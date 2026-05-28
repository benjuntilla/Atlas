package io.atlas.auth.kafka

import atlas.events.AuthTokenEvent
import io.atlas.auth.core.TokenClaims
import org.apache.kafka.clients.producer.KafkaProducer
import org.apache.kafka.clients.producer.Producer
import org.apache.kafka.clients.producer.ProducerConfig
import org.apache.kafka.clients.producer.ProducerRecord
import org.apache.kafka.common.serialization.ByteArraySerializer
import org.apache.kafka.common.serialization.StringSerializer
import org.slf4j.LoggerFactory
import java.time.Clock
import java.util.Properties

/**
 * Publishes [AuthTokenEvent] protobufs to `atlas.auth.tokens` so other
 * services (gateway, observability, consumers) can react to token lifecycle
 * changes. Topic payload format: protobuf, per Atlas locked decisions.
 *
 * Key is the `user_id` string so all events for a single user partition to
 * the same broker, preserving per-user ordering.
 */
interface AuthTokenEventPublisher {
    fun publishIssued(claims: TokenClaims, rawToken: String)
    fun publishRevoked(claims: TokenClaims, rawToken: String)
    fun close()
}

class AuthTokenProducer(
    private val producer: Producer<String, ByteArray>,
    private val topic: String = TOPIC,
    private val clock: Clock = Clock.systemUTC(),
) : AuthTokenEventPublisher {

    override fun publishIssued(claims: TokenClaims, rawToken: String) {
        publish(claims, rawToken, AuthTokenEvent.EventType.ISSUED)
    }

    override fun publishRevoked(claims: TokenClaims, rawToken: String) {
        publish(claims, rawToken, AuthTokenEvent.EventType.REVOKED)
    }

    private fun publish(claims: TokenClaims, rawToken: String, type: AuthTokenEvent.EventType) {
        val event = AuthTokenEvent.newBuilder()
            .setUserId(claims.userId.toString())
            .setTokenHash(sha256Hex(rawToken))
            .setSessionId(claims.sessionId.toString())
            .setEventType(type)
            .setOccurredAt(clock.instant().epochSecond)
            .build()
        val record = ProducerRecord(topic, claims.userId.toString(), event.toByteArray())
        producer.send(record) { _, exception ->
            if (exception != null) {
                // Surface but do not block the request path. The handoff doc
                // is explicit that token state of record is Postgres; Kafka is
                // a fanout channel for caches and downstream consumers. A
                // failed publish leaves the system in a degraded but correct
                // state (cache invalidations are missed; natural TTL recovers).
                LOG.warn("failed to publish AuthTokenEvent type={} userId={}", type, claims.userId, exception)
            }
        }
    }

    override fun close() {
        producer.close()
    }

    companion object {
        const val TOPIC = "atlas.auth.tokens"
        private val LOG = LoggerFactory.getLogger(AuthTokenProducer::class.java)

        fun build(bootstrapServers: String): AuthTokenProducer {
            val props = Properties().apply {
                put(ProducerConfig.BOOTSTRAP_SERVERS_CONFIG, bootstrapServers)
                put(ProducerConfig.KEY_SERIALIZER_CLASS_CONFIG, StringSerializer::class.java.name)
                put(ProducerConfig.VALUE_SERIALIZER_CLASS_CONFIG, ByteArraySerializer::class.java.name)
                put(ProducerConfig.ACKS_CONFIG, "all")
                put(ProducerConfig.ENABLE_IDEMPOTENCE_CONFIG, true)
                put(ProducerConfig.CLIENT_ID_CONFIG, "auth-service")
                put(ProducerConfig.LINGER_MS_CONFIG, 5)
            }
            return AuthTokenProducer(KafkaProducer(props))
        }
    }
}

/**
 * Stub publisher used by unit tests (and by App.kt if Kafka is intentionally
 * disabled for local-only experimentation). Records calls in-memory so tests
 * can assert on them.
 */
class RecordingAuthTokenPublisher : AuthTokenEventPublisher {
    data class Event(val type: AuthTokenEvent.EventType, val claims: TokenClaims, val rawToken: String)

    private val _events = mutableListOf<Event>()
    val events: List<Event> get() = synchronized(_events) { _events.toList() }

    override fun publishIssued(claims: TokenClaims, rawToken: String) {
        synchronized(_events) { _events += Event(AuthTokenEvent.EventType.ISSUED, claims, rawToken) }
    }

    override fun publishRevoked(claims: TokenClaims, rawToken: String) {
        synchronized(_events) { _events += Event(AuthTokenEvent.EventType.REVOKED, claims, rawToken) }
    }

    override fun close() {}
}
