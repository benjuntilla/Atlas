package io.atlas.payments.db

import io.atlas.payments.core.DuplicateIdempotencyKey
import io.atlas.payments.core.TransactionRepository
import io.atlas.payments.core.TransactionRunner
import io.atlas.payments.core.TxRecord
import io.atlas.payments.core.TxStatus
import io.atlas.payments.core.Wallet
import io.atlas.payments.core.WalletRepository
import org.jetbrains.exposed.exceptions.ExposedSQLException
import org.jetbrains.exposed.sql.ResultRow
import org.jetbrains.exposed.sql.SqlExpressionBuilder.plus
import org.jetbrains.exposed.sql.insert
import org.jetbrains.exposed.sql.selectAll
import org.jetbrains.exposed.sql.transactions.transaction
import org.jetbrains.exposed.sql.update
import java.time.Instant
import java.util.UUID

/**
 * Postgres-backed repositories. Each method opens an Exposed `transaction {}`.
 * Exposed nests these by joining the outermost transaction, so when
 * [ExposedTransactionRunner] wraps several calls in one `transaction {}` they
 * all commit (or roll back) together - the property the outbox pattern needs.
 */

private fun ResultRow.toWallet() = Wallet(
    id = this[Wallets.id],
    userId = this[Wallets.userId],
    balanceCents = this[Wallets.balanceCents],
    currency = this[Wallets.currency],
)

private fun ResultRow.toTxRecord() = TxRecord(
    id = this[Transactions.id],
    fromWallet = this[Transactions.fromWallet],
    toWallet = this[Transactions.toWallet],
    amountCents = this[Transactions.amountCents],
    status = this[Transactions.status],
    idempotencyKey = this[Transactions.idempotencyKey],
    rideId = this[Transactions.rideId],
    providerRef = this[Transactions.providerRef],
    idempotencyArgsHash = this[Transactions.idempotencyArgsHash],
    kind = this[Transactions.kind],
)

class ExposedWalletRepository : WalletRepository {
    override fun getOrCreateByUser(userId: UUID): Wallet = transaction {
        Wallets.selectAll().where { Wallets.userId eq userId }.singleOrNull()?.toWallet()
            ?: run {
                try {
                    Wallets.insert {
                        it[Wallets.userId] = userId
                        it[balanceCents] = 0
                        it[currency] = "USD"
                        it[updatedAt] = Instant.now()
                    }
                } catch (e: ExposedSQLException) {
                    // Concurrent create lost the race; ignore the unique
                    // violation and re-read the winning row below.
                    if (e.sqlState != "23505") throw e
                }
                Wallets.selectAll().where { Wallets.userId eq userId }.single().toWallet()
            }
    }

    override fun findByUser(userId: UUID): Wallet? = transaction {
        Wallets.selectAll().where { Wallets.userId eq userId }.singleOrNull()?.toWallet()
    }

    override fun findById(id: UUID): Wallet? = transaction {
        Wallets.selectAll().where { Wallets.id eq id }.singleOrNull()?.toWallet()
    }

    override fun adjustBalance(walletId: UUID, deltaCents: Long) {
        transaction {
            Wallets.update({ Wallets.id eq walletId }) {
                it[balanceCents] = balanceCents + deltaCents
                it[updatedAt] = Instant.now()
            }
        }
    }
}

class ExposedTransactionRepository : TransactionRepository {
    override fun findByIdempotencyKey(key: String): TxRecord? = transaction {
        Transactions.selectAll().where { Transactions.idempotencyKey eq key }.singleOrNull()?.toTxRecord()
    }

    override fun findById(id: UUID): TxRecord? = transaction {
        Transactions.selectAll().where { Transactions.id eq id }.singleOrNull()?.toTxRecord()
    }

    override fun insertPending(
        fromWallet: UUID?,
        toWallet: UUID?,
        amountCents: Long,
        idempotencyKey: String,
        rideId: UUID?,
        providerRef: String?,
        argsHash: String?,
        kind: String,
    ): TxRecord = transaction {
        try {
            val id = Transactions.insert {
                it[Transactions.fromWallet] = fromWallet
                it[Transactions.toWallet] = toWallet
                it[Transactions.amountCents] = amountCents
                it[status] = TxStatus.PENDING
                it[Transactions.idempotencyKey] = idempotencyKey
                it[Transactions.rideId] = rideId
                it[Transactions.providerRef] = providerRef
                it[idempotencyArgsHash] = argsHash
                it[Transactions.kind] = kind
                it[createdAt] = Instant.now()
            } get Transactions.id
            Transactions.selectAll().where { Transactions.id eq id }.single().toTxRecord()
        } catch (e: ExposedSQLException) {
            if (e.sqlState == "23505") throw DuplicateIdempotencyKey(idempotencyKey)
            throw e
        }
    }

    override fun markSettled(id: UUID, settledAt: Instant) {
        transaction {
            Transactions.update({ Transactions.id eq id }) {
                it[status] = TxStatus.SETTLED
                it[Transactions.settledAt] = settledAt
            }
        }
    }

    override fun markRefunded(id: UUID) {
        transaction {
            Transactions.update({ Transactions.id eq id }) {
                it[status] = TxStatus.REFUNDED
            }
        }
    }

    override fun markFailed(id: UUID, reason: String) {
        transaction {
            Transactions.update({ Transactions.id eq id }) {
                it[status] = TxStatus.FAILED
            }
        }
        // The reason is logged rather than stored: payments.transactions has
        // no column for it, and adding one to carry a provider string that
        // is only ever read by a human is not worth a migration. The
        // provider_ref on the row is what reconciliation actually needs.
        LOG.warn("transaction {} marked failed: {}", id, reason)
    }

    private companion object {
        private val LOG = org.slf4j.LoggerFactory.getLogger(ExposedTransactionRepository::class.java)
    }
}

/** Production [TransactionRunner]: one Exposed transaction per [run]. */
class ExposedTransactionRunner : TransactionRunner {
    override fun <T> run(block: () -> T): T = transaction { block() }
}
