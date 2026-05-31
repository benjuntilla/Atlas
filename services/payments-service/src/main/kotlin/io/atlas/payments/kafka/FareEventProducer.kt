package io.atlas.payments.kafka

import org.apache.kafka.clients.producer.KafkaProducer
import org.apache.kafka.clients.producer.Producer
import org.apache.kafka.clients.producer.ProducerConfig
import org.apache.kafka.clients.producer.ProducerRecord
import org.apache.kafka.common.serialization.ByteArraySerializer
import org.apache.kafka.common.serialization.StringSerializer
import java.util.Properties
import java.util.concurrent.TimeUnit

/**
 * Publishes already-encoded protobuf payloads (atlas.events.FareEvent) to
 * Kafka. Used only by the outbox dispatcher - the request path never touches
 * Kafka directly, it writes to the outbox table instead.
 *
 * [publishBlocking] waits for the broker ack so the dispatcher only marks a row
 * dispatched after the publish actually succeeds. This is what makes the outbox
 * "exactly-once-effective": a crash between publish and mark replays the row
 * (at-least-once to Kafka), and downstream consumers dedup on transaction_id.
 */
interface FareEventPublisher {
    fun publishBlocking(topic: String, key: String, payload: ByteArray)
    fun close()
}

class FareEventProducer(
    private val producer: Producer<String, ByteArray>,
    private val ackTimeoutMs: Long = 10_000,
) : FareEventPublisher {

    override fun publishBlocking(topic: String, key: String, payload: ByteArray) {
        val record = ProducerRecord(topic, key, payload)
        // .get() surfaces broker errors as an ExecutionException, which the
        // dispatcher catches per-row and records as a retryable failure.
        producer.send(record).get(ackTimeoutMs, TimeUnit.MILLISECONDS)
    }

    override fun close() {
        producer.close()
    }

    companion object {
        fun build(bootstrapServers: String): FareEventProducer {
            val props = Properties().apply {
                put(ProducerConfig.BOOTSTRAP_SERVERS_CONFIG, bootstrapServers)
                put(ProducerConfig.KEY_SERIALIZER_CLASS_CONFIG, StringSerializer::class.java.name)
                put(ProducerConfig.VALUE_SERIALIZER_CLASS_CONFIG, ByteArraySerializer::class.java.name)
                put(ProducerConfig.ACKS_CONFIG, "all")
                put(ProducerConfig.ENABLE_IDEMPOTENCE_CONFIG, true)
                put(ProducerConfig.CLIENT_ID_CONFIG, "payments-service")
                put(ProducerConfig.LINGER_MS_CONFIG, 5)
            }
            return FareEventProducer(KafkaProducer(props))
        }
    }
}

/**
 * In-memory publisher for tests. Records every publish so assertions can check
 * the FareEvent payloads the dispatcher emitted.
 */
class RecordingFareEventPublisher : FareEventPublisher {
    data class Published(val topic: String, val key: String, val payload: ByteArray)

    private val _published = mutableListOf<Published>()
    val published: List<Published> get() = synchronized(_published) { _published.toList() }

    override fun publishBlocking(topic: String, key: String, payload: ByteArray) {
        synchronized(_published) { _published += Published(topic, key, payload) }
    }

    override fun close() {}
}
