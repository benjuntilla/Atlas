package io.atlas.payments.core

import java.util.UUID

/**
 * Transport-agnostic domain models. The Exposed and in-memory repositories both
 * map their rows to these; the service logic never sees a ResultRow or a proto.
 */

data class Wallet(
    val id: UUID,
    val userId: UUID,
    val balanceCents: Long,
    val currency: String,
)

data class TxRecord(
    val id: UUID,
    val fromWallet: UUID?,
    val toWallet: UUID?,
    val amountCents: Long,
    val status: String,
    val idempotencyKey: String,
    val rideId: UUID?,
    val providerRef: String?,
    val idempotencyArgsHash: String?,
)

/** A pending outbox row claimed by the dispatcher for publishing. */
data class OutboxRow(
    val id: UUID,
    val transactionId: UUID?,
    val topic: String,
    val payload: ByteArray,
) {
    // ByteArray needs structural equals/hashCode for test assertions.
    override fun equals(other: Any?): Boolean {
        if (this === other) return true
        if (other !is OutboxRow) return false
        return id == other.id &&
            transactionId == other.transactionId &&
            topic == other.topic &&
            payload.contentEquals(other.payload)
    }

    override fun hashCode(): Int {
        var result = id.hashCode()
        result = 31 * result + (transactionId?.hashCode() ?: 0)
        result = 31 * result + topic.hashCode()
        result = 31 * result + payload.contentHashCode()
        return result
    }
}

// Transaction status constants (mirrors the CHECK constraint in 0030_payments.sql).
object TxStatus {
    const val PENDING = "pending"
    const val SETTLED = "settled"
    const val FAILED = "failed"
    const val REFUNDED = "refunded"
}
