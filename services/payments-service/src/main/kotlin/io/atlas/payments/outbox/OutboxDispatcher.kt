package io.atlas.payments.outbox

import io.atlas.payments.core.OutboxBackend
import io.atlas.payments.core.PaymentsMetrics
import io.atlas.payments.kafka.FareEventPublisher
import org.slf4j.LoggerFactory
import java.time.Duration
import java.util.concurrent.atomic.AtomicBoolean
import kotlin.concurrent.thread

/**
 * Drains the transactional outbox to Kafka. Runs on a timer in the background
 * and can also be triggered synchronously via [drainOnce] (the DrainOutbox RPC
 * uses this for deterministic flushes in tests and ops).
 *
 * Each pass delegates the claim/mark bookkeeping to an [OutboxBackend] (which
 * holds the FOR UPDATE SKIP LOCKED rows in one transaction); this class only
 * owns the polling loop and the publish callback.
 */
class OutboxDispatcher(
    private val backend: OutboxBackend,
    private val publisher: FareEventPublisher,
    private val pollInterval: Duration = Duration.ofSeconds(2),
    private val batchSize: Int = 100,
    private val metrics: PaymentsMetrics = PaymentsMetrics.NOOP,
) {
    private val running = AtomicBoolean(false)
    private var worker: Thread? = null

    /**
     * Runs a single drain pass. [maxRows] of 0 means "use the configured batch
     * size". Returns the number of rows successfully published to Kafka.
     */
    fun drainOnce(maxRows: Int = 0): Int {
        val limit = if (maxRows <= 0) batchSize else maxRows
        val dispatched = backend.drain(limit) { row ->
            // Key by transaction id so all events for one transaction land on
            // the same partition, preserving per-transaction ordering. Blocks
            // until the broker acks so a publish failure keeps the row pending.
            publisher.publishBlocking(row.topic, (row.transactionId ?: row.id).toString(), row.payload)
        }
        if (dispatched > 0) metrics.outboxDispatched(dispatched)
        return dispatched
    }

    fun start() {
        if (!running.compareAndSet(false, true)) return
        worker = thread(name = "outbox-dispatcher", isDaemon = true) {
            LOG.info("outbox dispatcher started; pollInterval={}ms batchSize={}", pollInterval.toMillis(), batchSize)
            while (running.get()) {
                try {
                    val n = drainOnce()
                    if (n > 0) LOG.debug("dispatched {} outbox rows", n)
                } catch (e: Exception) {
                    // A failed pass (e.g. Kafka unreachable) must not kill the
                    // loop; rows stay pending and the next tick retries.
                    LOG.warn("outbox drain pass failed; will retry", e)
                }
                try {
                    Thread.sleep(pollInterval.toMillis())
                } catch (e: InterruptedException) {
                    Thread.currentThread().interrupt()
                    break
                }
            }
            LOG.info("outbox dispatcher stopped")
        }
    }

    fun close() {
        running.set(false)
        worker?.interrupt()
        worker?.join(Duration.ofSeconds(5).toMillis())
    }

    companion object {
        private val LOG = LoggerFactory.getLogger(OutboxDispatcher::class.java)
    }
}
