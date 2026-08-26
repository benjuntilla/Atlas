package io.atlas.fare.core

import atlas.events.FareEvent
import java.util.UUID

/**
 * The three things [FareEventHandler] needs from the outside world, kept
 * behind interfaces so the handler's decision logic is testable without
 * Kafka, Postgres, or a running payments-service.
 */

/**
 * Money operations, always over gRPC to payments-service.
 *
 * This consumer never mutates wallets or transactions directly, even
 * though it has a database connection. Payments owns that invariant — the
 * balance check, the provider capture, the outbox write and the status
 * transition happen together in one transaction there — and a second
 * writer reaching into `payments.transactions` would quietly break it.
 * The consumer's database access is read-only apart from its own audit
 * table.
 */
interface PaymentsCommands {
    fun settle(projectId: UUID, transactionId: UUID): CommandResult
    fun refund(projectId: UUID, transactionId: UUID): CommandResult
}

/**
 * What came back from a money operation, flattened so the handler does
 * not need to know about gRPC status codes.
 */
sealed interface CommandResult {
    /** Applied, or already in the requested state. */
    data object Applied : CommandResult

    /**
     * Payments refused for a reason that will not change on retry — the
     * transaction is in the wrong state, or does not exist. Cancelling a
     * ride whose transaction is still PENDING lands here.
     */
    data class Rejected(val reason: String) : CommandResult

    /** The call failed in a way that may succeed later. */
    data class Unavailable(val reason: String) : CommandResult
}

/**
 * Resolving a ride to the transaction payments created for it.
 *
 * Needed because `FareEvent.transaction_id` is documented as "empty until
 * payments creates one": the RIDE_COMPLETED and RIDE_CANCELLED events
 * that drive settlement come from the developer's application, which
 * knows its ride id and nothing about Atlas transaction ids.
 */
interface TransactionLookup {
    fun findByRideId(projectId: UUID, rideId: UUID): UUID?
}

/** The payments.transaction_events audit log. */
interface AuditLog {
    /**
     * Record one event. Returns false if this exact event was already
     * recorded, which is how at-least-once redelivery stays invisible in
     * the log.
     */
    fun record(entry: AuditEntry): Boolean
}

data class AuditEntry(
    val projectId: UUID,
    val eventKey: String,
    val transactionId: UUID?,
    val rideId: UUID?,
    val eventType: FareEvent.EventType,
    val payloadJson: String,
)
