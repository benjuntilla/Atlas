package io.atlas.payments.db

import io.atlas.payments.core.OutboxBackend
import io.atlas.payments.core.OutboxRow
import io.atlas.payments.core.OutboxStore
import org.jetbrains.exposed.sql.SqlExpressionBuilder.plus
import org.jetbrains.exposed.sql.insert
import org.jetbrains.exposed.sql.transactions.transaction
import org.jetbrains.exposed.sql.update
import org.slf4j.LoggerFactory
import java.time.Instant
import java.util.UUID

/**
 * Write side of the outbox: inserts a row in whatever transaction is currently
 * open. Called from inside [io.atlas.payments.core.PaymentsService]'s
 * `runner.run {}` block so the event commits atomically with the wallet and
 * transaction changes.
 */
class ExposedOutboxStore : OutboxStore {
    override fun enqueue(transactionId: UUID?, topic: String, payload: ByteArray) {
        transaction {
            Outbox.insert {
                it[Outbox.transactionId] = transactionId
                it[Outbox.topic] = topic
                it[Outbox.payload] = payload
                it[createdAt] = Instant.now()
            }
        }
    }
}

/**
 * Drain side: claims pending rows with FOR UPDATE SKIP LOCKED, publishes each,
 * and marks them dispatched - all in ONE transaction so the row locks are held
 * until commit. A row whose publish throws is left pending with an incremented
 * attempt count and the error recorded, to be retried on the next pass.
 *
 * SKIP LOCKED is what lets multiple payments-service replicas drain the same
 * table concurrently without double-publishing or blocking each other.
 */
class ExposedOutboxBackend : OutboxBackend {
    override fun drain(limit: Int, publish: (OutboxRow) -> Unit): Int = transaction {
        val pending = claimPending(limit)
        var dispatched = 0
        for (row in pending) {
            try {
                publish(row)
                Outbox.update({ Outbox.id eq row.id }) {
                    it[dispatchedAt] = Instant.now()
                }
                dispatched++
            } catch (e: Exception) {
                LOG.warn("failed to publish outbox row {}; recording for retry", row.id, e)
                Outbox.update({ Outbox.id eq row.id }) {
                    it[attempts] = attempts + 1
                    it[lastError] = (e.message ?: e.javaClass.simpleName).take(1000)
                }
            }
        }
        dispatched
    }

    private fun org.jetbrains.exposed.sql.Transaction.claimPending(limit: Int): List<OutboxRow> {
        val rows = mutableListOf<OutboxRow>()
        // limit is a server-controlled Int; the only interpolated value, so the
        // statement is not exposed to injection.
        exec(
            """
            SELECT id, transaction_id, topic, payload
            FROM payments.outbox
            WHERE dispatched_at IS NULL
            ORDER BY created_at
            FOR UPDATE SKIP LOCKED
            LIMIT $limit
            """.trimIndent(),
        ) { rs ->
            while (rs.next()) {
                rows += OutboxRow(
                    id = rs.getObject("id", UUID::class.java),
                    transactionId = rs.getObject("transaction_id", UUID::class.java),
                    topic = rs.getString("topic"),
                    payload = rs.getBytes("payload"),
                )
            }
        }
        return rows
    }

    companion object {
        private val LOG = LoggerFactory.getLogger(ExposedOutboxBackend::class.java)
    }
}
