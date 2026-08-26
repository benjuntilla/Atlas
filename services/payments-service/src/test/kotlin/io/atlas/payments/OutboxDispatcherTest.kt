package io.atlas.payments

import atlas.events.FareEvent
import io.atlas.payments.core.FakePaymentProvider
import io.atlas.payments.core.PaymentsService
import io.atlas.payments.fakes.DirectTransactionRunner
import io.atlas.payments.fakes.InMemoryOutbox
import io.atlas.payments.fakes.InMemoryTransactionRepository
import io.atlas.payments.fakes.InMemoryWalletRepository
import io.atlas.payments.kafka.FareEventPublisher
import io.atlas.payments.kafka.RecordingFareEventPublisher
import io.atlas.payments.outbox.OutboxDispatcher
import java.util.UUID
import kotlin.test.Test
import kotlin.test.assertEquals

class OutboxDispatcherTest {

    // Two tenants, so the scoping can be asserted rather than assumed. A
    // suite that only ever used one project would pass just as happily
    // against repositories that ignored projectId entirely.
    private val projectA: UUID = UUID.fromString("11111111-1111-1111-1111-111111111111")
    private val projectB: UUID = UUID.fromString("22222222-2222-2222-2222-222222222222")
    private val wallets = InMemoryWalletRepository()
    private val transactions = InMemoryTransactionRepository()
    private val outbox = InMemoryOutbox()
    private val topic = "atlas.fare.events"

    private val service = PaymentsService(
        wallets = wallets,
        transactions = transactions,
        outbox = outbox,
        runner = DirectTransactionRunner(),
        provider = FakePaymentProvider(),
        fareTopic = topic,
    )

    private fun seedOneEvent() {
        service.initiate(
            projectA,
            UUID.randomUUID().toString(),
            UUID.randomUUID().toString(),
            1500,
            "key-${UUID.randomUUID()}",
            UUID.randomUUID().toString(),
        )
    }

    @Test
    fun `drainOnce publishes pending rows then leaves none`() {
        seedOneEvent()
        seedOneEvent()
        val publisher = RecordingFareEventPublisher()
        val dispatcher = OutboxDispatcher(outbox, publisher, batchSize = 100)

        val dispatched = dispatcher.drainOnce()

        assertEquals(2, dispatched)
        assertEquals(2, publisher.published.size)
        assertEquals(0, outbox.pendingCount())
        assertEquals(topic, publisher.published.first().topic)
        // The published bytes decode to a valid FareEvent.
        assertEquals(
            FareEvent.EventType.RIDE_ACCEPTED,
            FareEvent.parseFrom(publisher.published.first().payload).eventType,
        )

        // A second pass finds nothing.
        assertEquals(0, dispatcher.drainOnce())
    }

    @Test
    fun `a failed publish leaves the row pending for retry`() {
        seedOneEvent()
        val flaky = object : FareEventPublisher {
            override fun publishBlocking(topic: String, key: String, payload: ByteArray) {
                throw RuntimeException("kafka unreachable")
            }
            override fun close() {}
        }
        val dispatcher = OutboxDispatcher(outbox, flaky, batchSize = 100)

        val dispatched = dispatcher.drainOnce()

        assertEquals(0, dispatched)
        assertEquals(1, outbox.pendingCount())
        assertEquals(1, outbox.attemptsFor(outbox.all().first().id))
    }
}
