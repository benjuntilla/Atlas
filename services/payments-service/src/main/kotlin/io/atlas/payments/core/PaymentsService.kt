package io.atlas.payments.core

import atlas.events.FareEvent
import org.slf4j.LoggerFactory
import java.time.Clock
import java.util.UUID

/**
 * Payment business logic, free of any gRPC or persistence detail.
 *
 * The money-moving invariant: every transaction state change writes a
 * [FareEvent] to the outbox in the SAME [TransactionRunner.run] block as the
 * wallet and transaction mutations, so the event and the state change commit
 * atomically. The background dispatcher later drains the outbox to Kafka.
 *
 * Provider calls (authorize/capture/refund) happen OUTSIDE the transaction:
 * network I/O must never hold a Postgres row lock.
 *
 * Lifecycle:
 *   Deposit             -> provider.authorize + capture, credit wallet, TRANSACTION_SETTLED
 *   InitiateTransaction -> provider.authorize, pending tx, RIDE_ACCEPTED event
 *   SettleTransaction   -> provider.capture, move balances, TRANSACTION_SETTLED
 *   RefundTransaction   -> provider.refund, reverse balances, TRANSACTION_REFUNDED
 *
 * Deposit is the only way money enters the system. Without it every wallet
 * sits at zero and [settle] refuses to move funds that are not there, which
 * made the whole service unusable end to end.
 */
class PaymentsService(
    private val wallets: WalletRepository,
    private val transactions: TransactionRepository,
    private val outbox: OutboxStore,
    private val runner: TransactionRunner,
    private val provider: PaymentProvider,
    private val fareTopic: String,
    private val clock: Clock = Clock.systemUTC(),
    private val metrics: PaymentsMetrics = PaymentsMetrics.NOOP,
) {
    data class DepositResult(val transactionId: UUID, val status: String, val balanceCents: Long)
    data class InitiateResult(val transactionId: UUID, val status: String)
    data class SettleResult(val success: Boolean, val status: String)
    data class RefundResult(val success: Boolean)
    data class WalletBalance(val balanceCents: Long, val currency: String)

    /**
     * Add funds to a user's wallet from an external payment method.
     *
     * # Ordering, and the window it leaves open
     *
     * The sequence is: authorize -> record a pending row -> capture ->
     * credit and settle. The pending row is written BEFORE the capture on
     * purpose. Capture is the step that actually takes the customer's
     * money, so if the process dies immediately afterwards the row is
     * already on disk with its `provider_ref`, and a sweep over pending
     * deposits (indexed for exactly this in migration 0033) can ask the
     * provider what happened and finish the job.
     *
     * Reversing the order — capture first, then write — would lose that:
     * a crash would leave a charged card and no record of it anywhere in
     * this system. Money taken with no trace is the one outcome worth
     * contorting the code to avoid.
     *
     * The remaining window is small and recoverable rather than absent,
     * which is the honest position for a system that has to call a
     * network service and write a database in two separate steps.
     */
    fun deposit(
        userId: String,
        amountCents: Long,
        idempotencyKey: String,
    ): DepositResult {
        if (amountCents <= 0) throw PaymentError.InvalidAmount(amountCents)
        if (idempotencyKey.isBlank()) throw PaymentError.InvalidArgument("idempotency_key is required")
        val userUuid = parseUuid(userId, "user_id")

        // A deposit has no counterparty and no ride, so the hash covers the
        // payee and the amount. Reusing a key with a different amount is a
        // conflict, not a silent success returning the wrong transaction.
        val argsHash = idempotencyArgsHash(userId, "", amountCents, "")

        transactions.findByIdempotencyKey(idempotencyKey)?.let { existing ->
            if (existing.idempotencyArgsHash != argsHash) {
                throw PaymentError.IdempotencyConflict(idempotencyKey)
            }
            return DepositResult(
                existing.id,
                existing.status,
                walletBalance(userId).balanceCents,
            )
        }

        val auth = provider.authorize(amountCents, idempotencyKey)
        if (!auth.success) throw PaymentError.ProviderDeclined(auth.message ?: "authorize declined")

        val pending = try {
            runner.run {
                val wallet = wallets.getOrCreateByUser(userUuid)
                transactions.insertPending(
                    // No source wallet: this money comes from outside the
                    // platform. That is what makes it a deposit.
                    fromWallet = null,
                    toWallet = wallet.id,
                    amountCents = amountCents,
                    idempotencyKey = idempotencyKey,
                    rideId = null,
                    providerRef = auth.providerRef,
                    argsHash = argsHash,
                    kind = TxKind.DEPOSIT,
                )
            }
        } catch (e: DuplicateIdempotencyKey) {
            val winner = transactions.findByIdempotencyKey(idempotencyKey) ?: throw e
            if (winner.idempotencyArgsHash != argsHash) {
                throw PaymentError.IdempotencyConflict(idempotencyKey)
            }
            return DepositResult(winner.id, winner.status, walletBalance(userId).balanceCents)
        }

        val capture = provider.capture(auth.providerRef)
        if (!capture.success) {
            // The charge was refused, so leaving the row pending would have
            // the reconciliation sweep retry a capture the provider has
            // already declined.
            transactions.markFailed(pending.id, capture.message ?: "capture declined")
            metrics.depositFailed()
            throw PaymentError.ProviderDeclined(capture.message ?: "capture declined")
        }

        val balance = runner.run {
            val walletId = pending.toWallet
                ?: throw PaymentError.InvalidState("deposit has no destination wallet")
            wallets.adjustBalance(walletId, amountCents)
            transactions.markSettled(pending.id, clock.instant())
            outbox.enqueue(
                pending.id,
                fareTopic,
                // No ride is involved, so ride_id is empty. The event type is
                // TRANSACTION_SETTLED rather than a new one: consumers already
                // treat that as an audit record, and adding an enum value
                // would force every consumer to redeploy before deposits
                // could ship.
                fareEvent("", pending.id, FareEvent.EventType.TRANSACTION_SETTLED, amountCents),
            )
            wallets.findById(walletId)?.balanceCents ?: amountCents
        }

        metrics.depositSettled()
        LOG.info("deposit settled tx={} amount={}", pending.id, amountCents)
        return DepositResult(pending.id, TxStatus.SETTLED, balance)
    }

    fun initiate(
        fromUserId: String,
        toUserId: String,
        amountCents: Long,
        idempotencyKey: String,
        rideId: String,
    ): InitiateResult {
        if (amountCents <= 0) throw PaymentError.InvalidAmount(amountCents)
        if (idempotencyKey.isBlank()) throw PaymentError.InvalidArgument("idempotency_key is required")
        val fromUuid = parseUuid(fromUserId, "from_user_id")
        val toUuid = parseUuid(toUserId, "to_user_id")
        if (fromUuid == toUuid) {
            throw PaymentError.InvalidArgument("from_user_id and to_user_id must differ")
        }
        val rideUuid = if (rideId.isBlank()) null else parseUuid(rideId, "ride_id")
        val argsHash = idempotencyArgsHash(fromUserId, toUserId, amountCents, rideId)

        // Idempotent replay: same key + same args returns the existing tx.
        transactions.findByIdempotencyKey(idempotencyKey)?.let { existing ->
            if (existing.idempotencyArgsHash != argsHash) {
                throw PaymentError.IdempotencyConflict(idempotencyKey)
            }
            return InitiateResult(existing.id, existing.status)
        }

        // Authorize against the provider BEFORE opening the db transaction.
        val auth = provider.authorize(amountCents, idempotencyKey)
        if (!auth.success) throw PaymentError.ProviderDeclined(auth.message ?: "authorize declined")

        val txId = try {
            runner.run {
                val fromWallet = wallets.getOrCreateByUser(fromUuid)
                val toWallet = wallets.getOrCreateByUser(toUuid)
                val record = transactions.insertPending(
                    fromWallet = fromWallet.id,
                    toWallet = toWallet.id,
                    amountCents = amountCents,
                    idempotencyKey = idempotencyKey,
                    rideId = rideUuid,
                    providerRef = auth.providerRef,
                    argsHash = argsHash,
                )
                outbox.enqueue(
                    record.id,
                    fareTopic,
                    fareEvent(rideId, record.id, FareEvent.EventType.RIDE_ACCEPTED, amountCents),
                )
                record.id
            }
        } catch (e: DuplicateIdempotencyKey) {
            // Lost a race with a concurrent identical request. The duplicate-key
            // violation rolled the whole transaction back, so we re-read the
            // winner in a FRESH transaction (the aborted one cannot run queries).
            val winner = transactions.findByIdempotencyKey(idempotencyKey)
                ?: throw e
            if (winner.idempotencyArgsHash != argsHash) {
                throw PaymentError.IdempotencyConflict(idempotencyKey)
            }
            return InitiateResult(winner.id, winner.status)
        }
        metrics.transactionInitiated()
        return InitiateResult(txId, TxStatus.PENDING)
    }

    fun settle(transactionId: String): SettleResult {
        val txUuid = parseUuid(transactionId, "transaction_id")
        val tx = transactions.findById(txUuid)
            ?: throw PaymentError.TransactionNotFound(transactionId)
        when (tx.status) {
            TxStatus.SETTLED -> return SettleResult(true, TxStatus.SETTLED) // idempotent
            TxStatus.PENDING -> Unit
            else -> throw PaymentError.InvalidState("cannot settle a ${tx.status} transaction")
        }

        val capture = provider.capture(tx.providerRef ?: "")
        if (!capture.success) throw PaymentError.ProviderDeclined(capture.message ?: "capture declined")

        runner.run {
            val fromWalletId = tx.fromWallet
                ?: throw PaymentError.InvalidState("transaction has no source wallet")
            val fromWallet = wallets.findById(fromWalletId)
                ?: throw PaymentError.InvalidState("source wallet not found")
            if (fromWallet.balanceCents < tx.amountCents) {
                throw PaymentError.InsufficientFunds(fromWallet.id)
            }
            wallets.adjustBalance(fromWalletId, -tx.amountCents)
            tx.toWallet?.let { wallets.adjustBalance(it, tx.amountCents) }
            transactions.markSettled(txUuid, clock.instant())
            outbox.enqueue(
                txUuid,
                fareTopic,
                fareEvent(rideRef(tx), txUuid, FareEvent.EventType.TRANSACTION_SETTLED, tx.amountCents),
            )
        }
        metrics.transactionSettled()
        return SettleResult(true, TxStatus.SETTLED)
    }

    fun refund(transactionId: String): RefundResult {
        val txUuid = parseUuid(transactionId, "transaction_id")
        val tx = transactions.findById(txUuid)
            ?: throw PaymentError.TransactionNotFound(transactionId)
        when (tx.status) {
            TxStatus.REFUNDED -> return RefundResult(true) // idempotent
            TxStatus.SETTLED -> Unit
            else -> throw PaymentError.InvalidState("can only refund a settled transaction; status=${tx.status}")
        }

        val refund = provider.refund(tx.providerRef ?: "")
        if (!refund.success) throw PaymentError.ProviderDeclined(refund.message ?: "refund declined")

        runner.run {
            // Reverse the settle: money flows back from payee to payer.
            tx.fromWallet?.let { wallets.adjustBalance(it, tx.amountCents) }
            tx.toWallet?.let { wallets.adjustBalance(it, -tx.amountCents) }
            transactions.markRefunded(txUuid)
            outbox.enqueue(
                txUuid,
                fareTopic,
                fareEvent(rideRef(tx), txUuid, FareEvent.EventType.TRANSACTION_REFUNDED, tx.amountCents),
            )
        }
        metrics.transactionRefunded()
        return RefundResult(true)
    }

    fun walletBalance(userId: String): WalletBalance {
        val uuid = parseUuid(userId, "user_id")
        val wallet = wallets.findByUser(uuid)
        return if (wallet == null) {
            WalletBalance(0, "USD")
        } else {
            WalletBalance(wallet.balanceCents, wallet.currency)
        }
    }

    // --- helpers ----------------------------------------------------------

    private fun rideRef(tx: TxRecord): String = tx.rideId?.toString() ?: ""

    private fun fareEvent(
        rideId: String,
        transactionId: UUID,
        type: FareEvent.EventType,
        amountCents: Long,
    ): ByteArray =
        FareEvent.newBuilder()
            .setRideId(rideId)
            .setTransactionId(transactionId.toString())
            .setEventType(type)
            .setAmountCents(amountCents)
            .setOccurredAt(clock.instant().epochSecond)
            .build()
            .toByteArray()

    private fun parseUuid(value: String, field: String): UUID =
        try {
            UUID.fromString(value)
        } catch (e: IllegalArgumentException) {
            throw PaymentError.InvalidArgument("$field is not a valid UUID: $value")
        }

    companion object {
        private val LOG = LoggerFactory.getLogger(PaymentsService::class.java)
    }
}
