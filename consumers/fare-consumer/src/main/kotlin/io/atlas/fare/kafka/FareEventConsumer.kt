package io.atlas.fare.kafka

import atlas.events.FareEvent
import io.atlas.fare.core.FareEventHandler
import org.apache.kafka.clients.consumer.Consumer
import org.apache.kafka.clients.consumer.ConsumerConfig
import org.apache.kafka.clients.consumer.KafkaConsumer
import org.apache.kafka.common.TopicPartition
import org.apache.kafka.common.serialization.ByteArrayDeserializer
import org.apache.kafka.common.serialization.StringDeserializer
import org.slf4j.LoggerFactory
import java.time.Duration
import java.util.Properties
import java.util.concurrent.atomic.AtomicBoolean
import kotlin.concurrent.thread

/**
 * Polls `atlas.fare.events` and feeds each record to [FareEventHandler].
 *
 * # Group semantics
 *
 * A single shared group id, unlike auth-service's cache-invalidation
 * consumer which deliberately uses a unique group per instance to
 * broadcast. Settlement must happen once, so partitions are divided
 * across instances rather than every instance seeing every event.
 *
 * # Offsets
 *
 * Auto-commit is off. Offsets advance only past records the handler
 * reports as COMMIT. When a record comes back RETRY, the loop seeks back
 * to it and stops processing that partition for this poll — anything
 * after it on the same partition stays unprocessed, which preserves
 * per-ride ordering. Other partitions are unaffected.
 */
class FareEventConsumer(
    private val consumer: Consumer<String, ByteArray>,
    private val handler: FareEventHandler,
    private val topic: String,
    private val pollTimeout: Duration = Duration.ofSeconds(1),
) : AutoCloseable {

    private val running = AtomicBoolean(false)
    private var worker: Thread? = null

    fun start() {
        if (!running.compareAndSet(false, true)) return
        worker = thread(name = "fare-consumer", isDaemon = false) { runLoop() }
    }

    override fun close() {
        if (!running.compareAndSet(true, false)) return
        // wakeup() makes an in-flight poll() throw WakeupException so the
        // loop exits promptly instead of waiting out the poll timeout.
        consumer.wakeup()
        worker?.join(Duration.ofSeconds(10).toMillis())
    }

    private fun runLoop() {
        try {
            consumer.subscribe(listOf(topic))
            LOG.info("subscribed to {}", topic)
            while (running.get()) {
                val records = consumer.poll(pollTimeout)
                if (records.isEmpty) continue
                for (partition in records.partitions()) {
                    processPartition(partition, records.records(partition))
                }
            }
        } catch (e: org.apache.kafka.common.errors.WakeupException) {
            if (running.get()) throw e // a wakeup we did not ask for
            LOG.info("consumer woken for shutdown")
        } catch (e: Exception) {
            LOG.error("fare consumer loop died", e)
        } finally {
            try {
                consumer.close(Duration.ofSeconds(5))
            } catch (e: Exception) {
                LOG.warn("error closing consumer", e)
            }
            LOG.info("fare consumer stopped")
        }
    }

    private fun processPartition(
        partition: TopicPartition,
        records: List<org.apache.kafka.clients.consumer.ConsumerRecord<String, ByteArray>>,
    ) {
        for (record in records) {
            val event = decode(record.value())

            // A null event is an undecodable payload: a schema mismatch,
            // not a transient fault. It still has to be committed past —
            // skipping it without advancing the offset would redeliver it
            // forever, which is the exact partition-wedging this is meant
            // to avoid.
            if (event != null) {
                when (handler.handle(event)) {
                    FareEventHandler.Outcome.COMMIT -> Unit
                    FareEventHandler.Outcome.RETRY -> {
                        // Rewind to this record so it is redelivered, and
                        // stop here so later records on this partition are
                        // not applied out of order ahead of it.
                        consumer.seek(partition, record.offset())
                        LOG.warn("holding offset {} on {} for retry", record.offset(), partition)
                        return
                    }
                }
            }

            // Commit past this record only once it is fully handled (or
            // deliberately discarded).
            consumer.commitSync(
                mapOf(
                    partition to org.apache.kafka.clients.consumer.OffsetAndMetadata(
                        record.offset() + 1,
                    ),
                ),
            )
        }
    }

    private fun decode(payload: ByteArray?): FareEvent? {
        if (payload == null) return null
        return try {
            FareEvent.parseFrom(payload)
        } catch (e: Exception) {
            LOG.warn("failed to decode FareEvent; skipping", e)
            null
        }
    }

    companion object {
        private val LOG = LoggerFactory.getLogger(FareEventConsumer::class.java)

        fun build(
            bootstrapServers: String,
            groupId: String,
            handler: FareEventHandler,
            topic: String,
        ): FareEventConsumer {
            val props = Properties().apply {
                put(ConsumerConfig.BOOTSTRAP_SERVERS_CONFIG, bootstrapServers)
                put(ConsumerConfig.GROUP_ID_CONFIG, groupId)
                put(ConsumerConfig.KEY_DESERIALIZER_CLASS_CONFIG, StringDeserializer::class.java.name)
                put(
                    ConsumerConfig.VALUE_DESERIALIZER_CLASS_CONFIG,
                    ByteArrayDeserializer::class.java.name,
                )
                // Earliest, so a first deploy settles the backlog of rides
                // that completed before this consumer existed rather than
                // silently skipping them.
                put(ConsumerConfig.AUTO_OFFSET_RESET_CONFIG, "earliest")
                put(ConsumerConfig.ENABLE_AUTO_COMMIT_CONFIG, false)
                // Small batches: each record can trigger a gRPC call, and a
                // large batch would risk exceeding max.poll.interval.ms
                // while payments is slow.
                put(ConsumerConfig.MAX_POLL_RECORDS_CONFIG, 50)
            }
            return FareEventConsumer(KafkaConsumer(props), handler, topic)
        }
    }
}
