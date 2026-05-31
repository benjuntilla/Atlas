package io.atlas.payments.core

import java.time.Instant
import java.util.UUID

/**
 * Persistence contracts. Production implementations live in
 * [io.atlas.payments.db] (Exposed + Postgres). Tests use the in-memory fakes
 * so [PaymentsService] can be exercised without a database.
 */

interface WalletRepository {
    /** Returns the wallet for [userId], creating a zero-balance one if absent. */
    fun getOrCreateByUser(userId: UUID): Wallet
    fun findByUser(userId: UUID): Wallet?
    fun findById(id: UUID): Wallet?
    /** Adds [deltaCents] (which may be negative) to the wallet balance. */
    fun adjustBalance(walletId: UUID, deltaCents: Long)
}

interface TransactionRepository {
    fun findByIdempotencyKey(key: String): TxRecord?
    fun findById(id: UUID): TxRecord?
    /** Inserts a pending transaction. Throws [DuplicateIdempotencyKey] on conflict. */
    fun insertPending(
        fromWallet: UUID?,
        toWallet: UUID?,
        amountCents: Long,
        idempotencyKey: String,
        rideId: UUID?,
        providerRef: String?,
        argsHash: String?,
    ): TxRecord
    fun markSettled(id: UUID, settledAt: Instant)
    fun markRefunded(id: UUID)
}

/**
 * Write side of the transactional outbox. [enqueue] is called from inside the
 * same transaction as the wallet/transaction mutations so the event and the
 * state change commit atomically.
 */
interface OutboxStore {
    fun enqueue(transactionId: UUID?, topic: String, payload: ByteArray)
}

/**
 * Drain side of the outbox. [drain] claims up to [limit] pending rows, calls
 * [publish] for each, and marks them dispatched (or records the failure) - all
 * within a single transaction so the FOR UPDATE SKIP LOCKED claim holds until
 * commit. Returns the number successfully published.
 */
interface OutboxBackend {
    fun drain(limit: Int, publish: (OutboxRow) -> Unit): Int
}

/**
 * Runs a block inside a single database transaction so a group of repository
 * calls commit atomically. Production wraps Exposed's `transaction {}`; tests
 * run the block directly. Repository methods join the ambient transaction.
 */
interface TransactionRunner {
    fun <T> run(block: () -> T): T
}
