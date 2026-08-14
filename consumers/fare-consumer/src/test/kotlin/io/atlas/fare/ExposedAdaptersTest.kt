package io.atlas.fare

import atlas.events.FareEvent
import io.atlas.fare.core.AuditEntry
import io.atlas.fare.db.DatabaseBootstrap
import io.atlas.fare.db.ExposedAuditLog
import io.atlas.fare.db.ExposedTransactionLookup
import io.atlas.fare.db.TransactionEvents
import org.jetbrains.exposed.sql.insert
import org.jetbrains.exposed.sql.javatime.timestamp
import org.jetbrains.exposed.sql.selectAll
import org.jetbrains.exposed.sql.Table
import org.jetbrains.exposed.sql.transactions.transaction
import org.junit.jupiter.api.Assumptions.assumeTrue
import org.junit.jupiter.api.BeforeAll
import java.time.Instant
import java.util.UUID
import kotlin.test.Test
import kotlin.test.assertEquals
import kotlin.test.assertFalse
import kotlin.test.assertNull
import kotlin.test.assertTrue

/**
 * Database-backed tests for the Exposed adapters.
 *
 * These exist because the pure handler tests cannot catch the failures
 * these adapters are actually prone to: the custom `jsonb` column type
 * binding a String to a JSONB column, and `insertIgnore` translating to
 * ON CONFLICT DO NOTHING against the real unique index from migration
 * 0032. Both compile fine and fail at runtime on the first event.
 *
 * Skipped unless ATLAS_TEST_DATABASE_URL is set, so `./gradlew build`
 * stays green without a database. CI's integration job sets it. Run
 * locally with:
 *
 *     docker compose up -d postgres
 *     ATLAS_TEST_DATABASE_URL=postgres://atlas:atlas_dev@localhost:5432/atlas \
 *       ./gradlew :consumers:fare-consumer:test
 */
class ExposedAdaptersTest {

    /** Minimal writer for payments.transactions, to seed lookup fixtures. */
    private object TransactionsWrite : Table("payments.transactions") {
        val id = uuid("id")
        val amountCents = long("amount_cents")
        val status = text("status")
        val idempotencyKey = text("idempotency_key")
        val rideId = uuid("ride_id").nullable()
        val createdAt = timestamp("created_at")
        override val primaryKey = PrimaryKey(id)
    }

    companion object {
        private var connected = false

        @JvmStatic
        @BeforeAll
        fun connect() {
            val raw = System.getenv("ATLAS_TEST_DATABASE_URL") ?: return
            val withoutScheme = raw.removePrefix("postgres://").removePrefix("postgresql://")
            val atIdx = withoutScheme.lastIndexOf('@')
            val credentials = withoutScheme.substring(0, atIdx)
            val hostAndPath = withoutScheme.substring(atIdx + 1)
            val colonIdx = credentials.indexOf(':')
            DatabaseBootstrap.connect(
                jdbcUrl = "jdbc:postgresql://$hostAndPath",
                username = credentials.substring(0, colonIdx),
                password = credentials.substring(colonIdx + 1),
            )
            connected = true
        }
    }

    private fun requireDatabase() =
        assumeTrue(connected, "ATLAS_TEST_DATABASE_URL not set; skipping database tests")

    private fun entry(
        key: String = UUID.randomUUID().toString(),
        transactionId: UUID? = null,
        rideId: UUID? = null,
    ) = AuditEntry(
        eventKey = key,
        transactionId = transactionId,
        rideId = rideId,
        eventType = FareEvent.EventType.RIDE_COMPLETED,
        payloadJson = """{"ride_id":"abc","amount_cents":2500}""",
    )

    /**
     * The one that would have failed at runtime: payload is JSONB, and a
     * plain VARCHAR bind is rejected by Postgres.
     */
    @Test
    fun `audit rows insert into the jsonb payload column`() {
        requireDatabase()
        val key = UUID.randomUUID().toString()
        assertTrue(ExposedAuditLog().record(entry(key = key)))

        val stored = transaction {
            TransactionEvents.selectAll()
                .where { TransactionEvents.eventKey eq key }
                .single()[TransactionEvents.payload]
        }
        assertTrue(stored.contains("amount_cents"), "payload round-tripped as: $stored")
    }

    /** The dedup guarantee, against the real unique index. */
    @Test
    fun `recording the same event key twice inserts one row`() {
        requireDatabase()
        val key = UUID.randomUUID().toString()
        val log = ExposedAuditLog()

        assertTrue(log.record(entry(key = key)), "first insert should report recorded")
        assertFalse(log.record(entry(key = key)), "replay should report already-recorded")

        val count = transaction {
            TransactionEvents.selectAll().where { TransactionEvents.eventKey eq key }.count()
        }
        assertEquals(1, count)
    }

    @Test
    fun `lookup finds the transaction for a ride`() {
        requireDatabase()
        val rideId = UUID.randomUUID()
        val txId = UUID.randomUUID()
        transaction {
            TransactionsWrite.insert {
                it[id] = txId
                it[amountCents] = 2_500
                it[status] = "pending"
                it[idempotencyKey] = "test-${UUID.randomUUID()}"
                it[TransactionsWrite.rideId] = rideId
                it[createdAt] = Instant.now()
            }
        }

        assertEquals(txId, ExposedTransactionLookup().findByRideId(rideId))
    }

    @Test
    fun `lookup returns null for an unknown ride`() {
        requireDatabase()
        assertNull(ExposedTransactionLookup().findByRideId(UUID.randomUUID()))
    }

    /**
     * A refunded transaction is finished. If a ride was paid for twice —
     * refunded, then re-initiated — the live row is the one a completion
     * refers to, not the dead one.
     */
    @Test
    fun `lookup skips refunded transactions in favour of the live one`() {
        requireDatabase()
        val rideId = UUID.randomUUID()
        val refunded = UUID.randomUUID()
        val live = UUID.randomUUID()
        transaction {
            TransactionsWrite.insert {
                it[id] = refunded
                it[amountCents] = 2_500
                it[status] = "refunded"
                it[idempotencyKey] = "test-${UUID.randomUUID()}"
                it[TransactionsWrite.rideId] = rideId
                it[createdAt] = Instant.now().minusSeconds(60)
            }
            TransactionsWrite.insert {
                it[id] = live
                it[amountCents] = 2_500
                it[status] = "pending"
                it[idempotencyKey] = "test-${UUID.randomUUID()}"
                it[TransactionsWrite.rideId] = rideId
                it[createdAt] = Instant.now()
            }
        }

        assertEquals(live, ExposedTransactionLookup().findByRideId(rideId))
    }
}
