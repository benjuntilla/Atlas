package io.atlas.fare

import atlas.events.FareEvent
import io.atlas.fare.core.AuditEntry
import io.atlas.fare.core.AuditLog
import io.atlas.fare.core.CommandResult
import io.atlas.fare.core.FareEventHandler
import io.atlas.fare.core.PaymentsCommands
import io.atlas.fare.core.TransactionLookup
import java.util.UUID
import kotlin.test.Test
import kotlin.test.assertContentEquals
import kotlin.test.assertEquals
import kotlin.test.assertFalse
import kotlin.test.assertTrue

/**
 * The handler's whole job is deciding what each event means, so these
 * tests are about the reaction table rather than any I/O.
 */
class FareEventHandlerTest {

    // --- fakes ------------------------------------------------------------

    class FakePayments(
        private val settleResult: CommandResult = CommandResult.Applied,
        private val refundResult: CommandResult = CommandResult.Applied,
    ) : PaymentsCommands {
        val settled = mutableListOf<UUID>()
        val refunded = mutableListOf<UUID>()

        override fun settle(transactionId: UUID): CommandResult {
            settled += transactionId
            return settleResult
        }

        override fun refund(transactionId: UUID): CommandResult {
            refunded += transactionId
            return refundResult
        }
    }

    class FakeLookup(private val byRide: Map<UUID, UUID> = emptyMap()) : TransactionLookup {
        val queried = mutableListOf<UUID>()
        override fun findByRideId(rideId: UUID): UUID? {
            queried += rideId
            return byRide[rideId]
        }
    }

    class FakeAudit : AuditLog {
        val entries = mutableListOf<AuditEntry>()
        private val keys = mutableSetOf<String>()
        override fun record(entry: AuditEntry): Boolean {
            entries += entry
            return keys.add(entry.eventKey)
        }
    }

    private val rideId = UUID.randomUUID()
    private val txId = UUID.randomUUID()

    private fun event(
        type: FareEvent.EventType,
        transactionId: String = txId.toString(),
        ride: String = rideId.toString(),
        occurredAt: Long = 1_700_000_000,
    ): FareEvent = FareEvent.newBuilder()
        .setRideId(ride)
        .setTransactionId(transactionId)
        .setEventType(type)
        .setAmountCents(2_500)
        .setOccurredAt(occurredAt)
        .build()

    private fun handler(
        payments: PaymentsCommands = FakePayments(),
        lookup: TransactionLookup = FakeLookup(),
        audit: AuditLog = FakeAudit(),
    ) = FareEventHandler(payments, lookup, audit)

    // --- the reaction table ----------------------------------------------

    @Test
    fun `ride completed settles the transaction`() {
        val payments = FakePayments()
        val audit = FakeAudit()
        val outcome = handler(payments, audit = audit)
            .handle(event(FareEvent.EventType.RIDE_COMPLETED))

        assertEquals(FareEventHandler.Outcome.COMMIT, outcome)
        assertContentEquals(listOf(txId), payments.settled)
        assertTrue(payments.refunded.isEmpty())
        assertEquals(1, audit.entries.size)
    }

    @Test
    fun `ride cancelled refunds the transaction`() {
        val payments = FakePayments()
        handler(payments).handle(event(FareEvent.EventType.RIDE_CANCELLED))

        assertContentEquals(listOf(txId), payments.refunded)
        assertTrue(payments.settled.isEmpty())
    }

    /**
     * The loop guard. Payments publishes TRANSACTION_SETTLED from inside
     * settle(), onto the topic this consumer reads. Acting on it would
     * settle in response to having settled, forever.
     */
    @Test
    fun `transaction settled is audited but never acted on`() {
        val payments = FakePayments()
        val audit = FakeAudit()
        val outcome = handler(payments, audit = audit)
            .handle(event(FareEvent.EventType.TRANSACTION_SETTLED))

        assertEquals(FareEventHandler.Outcome.COMMIT, outcome)
        assertTrue(
            payments.settled.isEmpty() && payments.refunded.isEmpty(),
            "acting on payments' own acknowledgement would loop through Kafka",
        )
        assertEquals(1, audit.entries.size)
    }

    @Test
    fun `transaction refunded is audited but never acted on`() {
        val payments = FakePayments()
        handler(payments).handle(event(FareEvent.EventType.TRANSACTION_REFUNDED))
        assertTrue(payments.settled.isEmpty() && payments.refunded.isEmpty())
    }

    @Test
    fun `ride accepted is audited but never acted on`() {
        val payments = FakePayments()
        handler(payments).handle(event(FareEvent.EventType.RIDE_ACCEPTED))
        assertTrue(payments.settled.isEmpty() && payments.refunded.isEmpty())
    }

    @Test
    fun `only the two ride outcome events require action`() {
        assertTrue(FareEventHandler.requiresAction(FareEvent.EventType.RIDE_COMPLETED))
        assertTrue(FareEventHandler.requiresAction(FareEvent.EventType.RIDE_CANCELLED))
        assertFalse(FareEventHandler.requiresAction(FareEvent.EventType.RIDE_ACCEPTED))
        assertFalse(FareEventHandler.requiresAction(FareEvent.EventType.TRANSACTION_SETTLED))
        assertFalse(FareEventHandler.requiresAction(FareEvent.EventType.TRANSACTION_REFUNDED))
        assertFalse(FareEventHandler.requiresAction(FareEvent.EventType.UNKNOWN))
    }

    // --- resolving the target transaction --------------------------------

    /**
     * The realistic path for an app-produced completion: the app knows
     * its ride id and nothing about Atlas transaction ids, so the event
     * carries an empty transaction_id.
     */
    @Test
    fun `an empty transaction id is resolved through the ride id`() {
        val payments = FakePayments()
        val lookup = FakeLookup(mapOf(rideId to txId))

        handler(payments, lookup)
            .handle(event(FareEvent.EventType.RIDE_COMPLETED, transactionId = ""))

        assertContentEquals(listOf(rideId), lookup.queried)
        assertContentEquals(listOf(txId), payments.settled)
    }

    @Test
    fun `an explicit transaction id skips the lookup`() {
        val lookup = FakeLookup(mapOf(rideId to UUID.randomUUID()))
        val payments = FakePayments()

        handler(payments, lookup).handle(event(FareEvent.EventType.RIDE_COMPLETED))

        assertTrue(lookup.queried.isEmpty(), "no lookup needed when the event names the transaction")
        assertContentEquals(listOf(txId), payments.settled)
    }

    /**
     * A ride that completes without a transaction is the application's
     * business — an unpaid ride — not a fault here. Retrying forever
     * would wedge the partition.
     */
    @Test
    fun `an unresolvable ride is audited and committed rather than retried`() {
        val payments = FakePayments()
        val audit = FakeAudit()
        val outcome = handler(payments, FakeLookup(), audit)
            .handle(event(FareEvent.EventType.RIDE_COMPLETED, transactionId = ""))

        assertEquals(FareEventHandler.Outcome.COMMIT, outcome)
        assertTrue(payments.settled.isEmpty())
        assertEquals(1, audit.entries.size)
    }

    // --- failure handling -------------------------------------------------

    /**
     * Cancelling a ride whose transaction is still PENDING: payments
     * cannot refund what it never captured. That is a permanent no, so
     * the offset must advance.
     */
    @Test
    fun `a rejection commits rather than retrying`() {
        val payments = FakePayments(
            refundResult = CommandResult.Rejected("can only refund a settled transaction"),
        )
        val audit = FakeAudit()
        val outcome = handler(payments, audit = audit)
            .handle(event(FareEvent.EventType.RIDE_CANCELLED))

        assertEquals(FareEventHandler.Outcome.COMMIT, outcome)
        assertEquals(1, audit.entries.size)
    }

    /**
     * Payments being down must NOT advance the offset, or the settlement
     * is lost silently.
     */
    @Test
    fun `an unavailable payments service holds the offset`() {
        val payments = FakePayments(settleResult = CommandResult.Unavailable("UNAVAILABLE"))
        val audit = FakeAudit()
        val outcome = handler(payments, audit = audit)
            .handle(event(FareEvent.EventType.RIDE_COMPLETED))

        assertEquals(FareEventHandler.Outcome.RETRY, outcome)
        assertTrue(
            audit.entries.isEmpty(),
            "an event that will be replayed must not be logged as handled",
        )
    }

    // --- dedup ------------------------------------------------------------

    @Test
    fun `the same event yields the same key`() {
        val a = FareEventHandler.eventKey(event(FareEvent.EventType.RIDE_COMPLETED))
        val b = FareEventHandler.eventKey(event(FareEvent.EventType.RIDE_COMPLETED))
        assertEquals(a, b)
    }

    @Test
    fun `different events yield different keys`() {
        val completed = FareEventHandler.eventKey(event(FareEvent.EventType.RIDE_COMPLETED))
        val cancelled = FareEventHandler.eventKey(event(FareEvent.EventType.RIDE_CANCELLED))
        val later = FareEventHandler.eventKey(
            event(FareEvent.EventType.RIDE_COMPLETED, occurredAt = 1_700_000_999),
        )
        assertEquals(3, setOf(completed, cancelled, later).size)
    }

    /**
     * Redelivery must repeat the (idempotent) money operation but must
     * not add a second audit row.
     */
    @Test
    fun `a replayed event does not duplicate its audit row`() {
        val payments = FakePayments()
        val audit = FakeAudit()
        val h = handler(payments, audit = audit)
        val e = event(FareEvent.EventType.RIDE_COMPLETED)

        h.handle(e)
        h.handle(e)

        assertEquals(2, payments.settled.size, "settle is idempotent upstream, so repeating is fine")
        assertEquals(
            1,
            audit.entries.map { it.eventKey }.distinct().size,
            "both attempts share one key, so the unique index collapses them",
        )
    }

    @Test
    fun `the audit payload is valid json with the event fields`() {
        val json = FareEventHandler.payloadJson(event(FareEvent.EventType.RIDE_COMPLETED))
        assertTrue(json.startsWith("{") && json.endsWith("}"), "got $json")
        assertTrue(json.contains("\"event_type\":\"RIDE_COMPLETED\""), "got $json")
        assertTrue(json.contains("\"amount_cents\":2500"), "got $json")
        assertTrue(json.contains("\"ride_id\":\"$rideId\""), "got $json")
    }

    /**
     * The payload is assembled by string concatenation from values that
     * arrived over Kafka, so a quote in one of them must not be able to
     * break out of its JSON string.
     */
    @Test
    fun `the audit payload escapes quotes in event fields`() {
        val nasty = event(FareEvent.EventType.RIDE_COMPLETED, ride = """a" ,"injected":"x""")
        val json = FareEventHandler.payloadJson(nasty)
        assertFalse(json.contains(""""injected":"x""""), "quote was not escaped: $json")
        assertTrue(json.contains("""\""""), "expected an escaped quote in $json")
    }
}
