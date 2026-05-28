package io.atlas.auth.kafka

import java.security.MessageDigest

/**
 * SHA-256 of a JWT, hex-encoded. The Kafka [AuthTokenEvent] carries this
 * value rather than the raw token so the topic does not become a leak
 * channel: anyone with read access to atlas.auth.tokens learns *which* token
 * changed state without being able to use the token.
 */
internal fun sha256Hex(input: String): String {
    val digest = MessageDigest.getInstance("SHA-256").digest(input.toByteArray(Charsets.UTF_8))
    return buildString(digest.size * 2) {
        for (b in digest) {
            val v = b.toInt() and 0xFF
            append(HEX[v ushr 4])
            append(HEX[v and 0x0F])
        }
    }
}

private val HEX = "0123456789abcdef".toCharArray()
