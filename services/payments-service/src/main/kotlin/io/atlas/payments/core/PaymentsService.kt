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
 *   InitiateTransaction -> provider.authorize, pending tx, RIDE_ACCEPTED event
 *   SettleTransaction   -> provider.capture, move balances, TRANSACTION_SETTLED
 *   RefundTransaction   -> provider.refund, reverse balances, TRANSACTION_REFUNDED
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
    data class InitiateResult(val transactionId: UUID, val status: String)
    data class SettleResult(val success: Boolean, val status: String)
    data class RefundResult(val success: Boolean)
    data class WalletBalance(val balanceCents: Long, val currency: String)

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
