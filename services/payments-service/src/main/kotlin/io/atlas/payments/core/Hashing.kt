package io.atlas.payments.core

import java.security.MessageDigest

/** Lowercase hex SHA-256 of [input]. */
fun sha256Hex(input: String): String {
    val digest = MessageDigest.getInstance("SHA-256").digest(input.toByteArray(Charsets.UTF_8))
    val sb = StringBuilder(digest.size * 2)
    for (b in digest) {
        sb.append(Character.forDigit((b.toInt() shr 4) and 0xF, 16))
        sb.append(Character.forDigit(b.toInt() and 0xF, 16))
    }
    return sb.toString()
}

/**
 * Hash of the business arguments behind an idempotency key. Stored on the
 * transaction row so a reused key with DIFFERENT args is rejected rather than
 * silently returning the wrong transaction (see 0031_payments_outbox.sql).
 */
fun idempotencyArgsHash(
    fromUserId: String,
    toUserId: String,
    amountCents: Long,
    rideId: String,
): String = sha256Hex("$fromUserId|$toUserId|$amountCents|$rideId")
