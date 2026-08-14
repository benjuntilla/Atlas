package io.atlas.fare.core

import atlas.events.FareEvent
import org.slf4j.LoggerFactory
import java.security.MessageDigest
import java.util.UUID

/**
 * Decides what each fare event means and carries it out.
 *
 * # The reaction table
 *
 * | Event                | Action                          |
 * |----------------------|---------------------------------|
 * | RIDE_ACCEPTED        | audit only                      |
 * | RIDE_COMPLETED       | SettleTransaction               |
 * | RIDE_CANCELLED       | RefundTransaction               |
 * | TRANSACTION_SETTLED  | audit only                      |
 * | TRANSACTION_REFUNDED | audit only                      |
 *
 * The two TRANSACTION_* rows are the important ones. Payments publishes
 * those itself from inside `settle` and `refund`, onto the same topic
 * this consumer reads. Treating them as instructions would mean settling
 * in response to having settled — an infinite loop through Kafka. They
 * are acknowledgements, so they are only ever recorded.
 *
 * RIDE_ACCEPTED is likewise payments' own announcement that it created a
 * transaction; the transaction already exists by then.
 *
 * # Ordering: act first, then audit
 *
 * A replayed event must not skip its action. If the audit row were
 * written first, a crash in between would leave a log entry claiming the
 * event was handled while the settlement never happened, and the replay
 * would dedup itself into doing nothing. Acting first is safe in the
 * other direction because the money operations are idempotent upstream:
 * settling an already-settled transaction returns success without moving
 * anything.
 */
class FareEventHandler(
    private val payments: PaymentsCommands,
    private val lookup: TransactionLookup,
    private val audit: AuditLog,
    private val metrics: FareMetrics = FareMetrics.NOOP,
) {

    /** Whether the Kafka offset may advance past this event. */
    enum class Outcome { COMMIT, RETRY }

    fun handle(event: FareEvent): Outcome {
        val rideId = parseUuidOrNull(event.rideId)
        val eventTransactionId = parseUuidOrNull(event.transactionId)

        if (requiresAction(event.eventType)) {
            val target = eventTransactionId ?: rideId?.let { lookup.findByRideId(it) }
            if (target == null) {
                // Nothing to act on. Either the event named neither a
                // transaction nor a ride, or the ride has no transaction —
                // a ride completing that was never paid for. Both are the
                // application's business, not a fault here, so record and
                // move on rather than retrying forever.
                LOG.warn(
                    "no transaction for {} event (ride_id='{}', transaction_id='{}'); auditing only",
                    event.eventType, event.rideId, event.transactionId,
                )
                metrics.unresolved()
                return auditAndCommit(event, null, rideId)
            }

            when (val result = act(event.eventType, target)) {
                is CommandResult.Applied ->
                    if (event.eventType == FareEvent.EventType.RIDE_COMPLETED) {
                        metrics.settled()
                    } else {
                        metrics.refunded()
                    }

                is CommandResult.Rejected ->
                    // Expected in normal operation: cancelling a ride whose
                    // transaction is still PENDING cannot be refunded,
                    // because there is nothing captured to reverse. Record
                    // it and let the offset advance — retrying cannot help.
                {
                    LOG.info(
                        "payments rejected {} for transaction {}: {}",
                        event.eventType, target, result.reason,
                    )
                    metrics.rejected()
                }

                is CommandResult.Unavailable -> {
                    // Hold the offset. Redelivery is the recovery path.
                    LOG.warn(
                        "payments unavailable for {} on transaction {}: {}; will retry",
                        event.eventType, target, result.reason,
                    )
                    metrics.retried()
                    return Outcome.RETRY
                }
            }
            return auditAndCommit(event, target, rideId)
        }

        return auditAndCommit(event, eventTransactionId, rideId)
    }

    private fun act(type: FareEvent.EventType, transactionId: UUID): CommandResult =
        when (type) {
            FareEvent.EventType.RIDE_COMPLETED -> payments.settle(transactionId)
            FareEvent.EventType.RIDE_CANCELLED -> payments.refund(transactionId)
            else -> CommandResult.Applied
        }

    private fun auditAndCommit(event: FareEvent, transactionId: UUID?, rideId: UUID?): Outcome {
        val recorded = audit.record(
            AuditEntry(
                eventKey = eventKey(event),
                transactionId = transactionId,
                rideId = rideId,
                eventType = event.eventType,
                payloadJson = payloadJson(event),
            ),
        )
        if (!recorded) {
            LOG.debug("duplicate fare event ignored: {}", event.eventType)
            metrics.duplicate()
        }
        return Outcome.COMMIT
    }

    companion object {
        private val LOG = LoggerFactory.getLogger(FareEventHandler::class.java)

        /**
         * Events that move money. Everything else is an acknowledgement
         * of something that already happened.
         */
        fun requiresAction(type: FareEvent.EventType): Boolean = when (type) {
            FareEvent.EventType.RIDE_COMPLETED,
            FareEvent.EventType.RIDE_CANCELLED,
            -> true

            FareEvent.EventType.RIDE_ACCEPTED,
            FareEvent.EventType.TRANSACTION_SETTLED,
            FareEvent.EventType.TRANSACTION_REFUNDED,
            FareEvent.EventType.UNKNOWN,
            FareEvent.EventType.UNRECOGNIZED,
            -> false
        }

        /**
         * Stable identity for one logical event, used to dedup the audit
         * log across redeliveries.
         *
         * Includes occurred_at so two genuinely distinct events of the
         * same type on the same ride — a retry of a cancelled ride, say —
         * are not collapsed into one row.
         */
        fun eventKey(event: FareEvent): String {
            val material =
                "${event.rideId}|${event.transactionId}|${event.eventType.number}|${event.occurredAt}"
            val digest = MessageDigest.getInstance("SHA-256").digest(material.toByteArray())
            return digest.joinToString("") { "%02x".format(it) }
        }

        fun payloadJson(event: FareEvent): String = buildString {
            append("{")
            append("\"ride_id\":\"").append(escape(event.rideId)).append("\",")
            append("\"transaction_id\":\"").append(escape(event.transactionId)).append("\",")
            append("\"event_type\":\"").append(event.eventType.name).append("\",")
            append("\"amount_cents\":").append(event.amountCents).append(",")
            append("\"occurred_at\":").append(event.occurredAt)
            append("}")
        }

        // ride_id and transaction_id are UUID strings in practice, but they
        // arrive from Kafka and this builds JSON by hand, so quotes and
        // backslashes get escaped rather than trusted.
        private fun escape(s: String): String =
            s.replace("\\", "\\\\").replace("\"", "\\\"")

        private fun parseUuidOrNull(value: String): UUID? =
            if (value.isBlank()) null
            else try {
                UUID.fromString(value)
            } catch (e: IllegalArgumentException) {
                null
            }
    }
}
