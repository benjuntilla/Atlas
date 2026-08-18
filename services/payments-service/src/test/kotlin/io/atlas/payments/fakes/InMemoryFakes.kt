package io.atlas.payments.fakes

import io.atlas.payments.core.DuplicateIdempotencyKey
import io.atlas.payments.core.OutboxBackend
import io.atlas.payments.core.OutboxRow
import io.atlas.payments.core.OutboxStore
import io.atlas.payments.core.TransactionRepository
import io.atlas.payments.core.TransactionRunner
import io.atlas.payments.core.TxRecord
import io.atlas.payments.core.TxStatus
import io.atlas.payments.core.Wallet
import io.atlas.payments.core.WalletRepository
import java.time.Instant
import java.util.UUID

/**
 * In-memory repository fakes so [io.atlas.payments.core.PaymentsService] and the
 * [io.atlas.payments.outbox.OutboxDispatcher] can be unit-tested without a
 * database. They mirror the semantics of the Exposed implementations.
 */

/** Runs the block directly; tests do not need real transaction isolation. */
class DirectTransactionRunner : TransactionRunner {
    override fun <T> run(block: () -> T): T = block()
}

class InMemoryWalletRepository : WalletRepository {
    private val byId = linkedMapOf<UUID, Wallet>()
    private val userToId = linkedMapOf<UUID, UUID>()

    @Synchronized
    override fun getOrCreateByUser(userId: UUID): Wallet {
        userToId[userId]?.let { return byId.getValue(it) }
        val wallet = Wallet(UUID.randomUUID(), userId, 0, "USD")
        byId[wallet.id] = wallet
        userToId[userId] = wallet.id
        return wallet
    }

    @Synchronized
    override fun findByUser(userId: UUID): Wallet? = userToId[userId]?.let { byId[it] }

    @Synchronized
    override fun findById(id: UUID): Wallet? = byId[id]

    @Synchronized
    override fun adjustBalance(walletId: UUID, deltaCents: Long) {
        val wallet = byId.getValue(walletId)
        byId[walletId] = wallet.copy(balanceCents = wallet.balanceCents + deltaCents)
    }
}

class InMemoryTransactionRepository : TransactionRepository {
    private val byId = linkedMapOf<UUID, TxRecord>()
    private val keyToId = linkedMapOf<String, UUID>()

    @Synchronized
    override fun findByIdempotencyKey(key: String): TxRecord? = keyToId[key]?.let { byId[it] }

    @Synchronized
    override fun findById(id: UUID): TxRecord? = byId[id]

    @Synchronized
    override fun insertPending(
        fromWallet: UUID?,
        toWallet: UUID?,
        amountCents: Long,
        idempotencyKey: String,
        rideId: UUID?,
        providerRef: String?,
        argsHash: String?,
        kind: String,
    ): TxRecord {
        if (keyToId.containsKey(idempotencyKey)) throw DuplicateIdempotencyKey(idempotencyKey)
        val record = TxRecord(
            id = UUID.randomUUID(),
            fromWallet = fromWallet,
            toWallet = toWallet,
            amountCents = amountCents,
            status = TxStatus.PENDING,
            idempotencyKey = idempotencyKey,
            rideId = rideId,
            providerRef = providerRef,
            idempotencyArgsHash = argsHash,
            kind = kind,
        )
        byId[record.id] = record
        keyToId[idempotencyKey] = record.id
        return record
    }

    @Synchronized
    override fun markSettled(id: UUID, settledAt: Instant) {
        byId[id] = byId.getValue(id).copy(status = TxStatus.SETTLED)
    }

    @Synchronized
    override fun markRefunded(id: UUID) {
        byId[id] = byId.getValue(id).copy(status = TxStatus.REFUNDED)
    }

    @Synchronized
    override fun markFailed(id: UUID, reason: String) {
        byId[id] = byId.getValue(id).copy(status = TxStatus.FAILED)
    }
}

/**
 * In-memory outbox implementing both the write side ([OutboxStore]) and the
 * drain side ([OutboxBackend]) over one shared list, so a test can enqueue via
 * the service and drain via the dispatcher.
 */
class InMemoryOutbox : OutboxStore, OutboxBackend {
    private val rows = mutableListOf<OutboxRow>()
    private val dispatched = mutableSetOf<UUID>()
    private val attempts = linkedMapOf<UUID, Int>()

    @Synchronized
    override fun enqueue(transactionId: UUID?, topic: String, payload: ByteArray) {
        rows += OutboxRow(UUID.randomUUID(), transactionId, topic, payload)
    }

    @Synchronized
    override fun drain(limit: Int, publish: (OutboxRow) -> Unit): Int {
        val pending = rows.filter { it.id !in dispatched }.take(limit)
        var count = 0
        for (row in pending) {
            try {
                publish(row)
                dispatched += row.id
                count++
            } catch (e: Exception) {
                attempts[row.id] = (attempts[row.id] ?: 0) + 1
            }
        }
        return count
    }

    @Synchronized
    fun pendingCount(): Int = rows.count { it.id !in dispatched }

    @Synchronized
    fun all(): List<OutboxRow> = rows.toList()

    @Synchronized
    fun attemptsFor(id: UUID): Int = attempts[id] ?: 0
}
