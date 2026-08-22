package io.atlas.payments.core

import java.security.MessageDigest
import java.time.Clock
import java.time.Duration
import javax.crypto.Mac
import javax.crypto.spec.SecretKeySpec

/**
 * HMAC webhook signature verification.
 *
 * # Why this is provider-independent
 *
 * Stripe, GitHub, Shopify, Adyen's HMAC mode and most others all sign the
 * same way: HMAC-SHA256 over the raw body with a shared secret, sent in a
 * header. The differences are in encoding and header layout, not in the
 * scheme. So the hard parts — constant-time comparison, replay rejection,
 * signing the RAW body — live here once, and a provider adapter only has
 * to describe its header format.
 *
 * # What makes a verifier correct
 *
 * Three things, and each has been the subject of real CVEs:
 *
 *  1. **The raw body.** Signatures cover the exact bytes sent. Parsing to
 *     JSON and re-serialising changes key order and whitespace, and the
 *     signature no longer matches — or worse, matches a document that is
 *     not what was signed.
 *  2. **Constant-time comparison.** `==` on strings returns early at the
 *     first differing byte, which leaks how much of a guessed signature
 *     was right. That is enough to forge one byte at a time.
 *  3. **A timestamp window.** A signature stays valid forever otherwise,
 *     so anyone who captures one legitimate webhook can replay it —
 *     against a payments system, repeatedly.
 */
class HmacWebhookVerifier(
    private val secret: String,
    /**
     * How far out of date a signed timestamp may be.
     *
     * Five minutes is the usual choice and it is a genuine trade-off: too
     * tight and legitimate deliveries fail on clock skew or a slow retry,
     * too loose and a captured signature stays replayable for that long.
     */
    private val tolerance: Duration = Duration.ofMinutes(5),
    private val clock: Clock = Clock.systemUTC(),
) {
    init {
        require(secret.isNotBlank()) {
            "webhook signing secret must not be blank; an empty secret verifies nothing"
        }
    }

    /**
     * Verify a `t=<unix>,v1=<hex>` style signature header.
     *
     * Returns false for every failure rather than throwing, and never
     * reports WHICH check failed. A caller cannot act differently on
     * "wrong signature" versus "too old", and telling an attacker which
     * half they got right is free information.
     */
    fun verify(payload: String, header: String?): Boolean {
        if (header.isNullOrBlank()) return false

        val parts = header.split(',')
            .mapNotNull { part ->
                val idx = part.indexOf('=')
                if (idx <= 0) null else part.substring(0, idx).trim() to part.substring(idx + 1).trim()
            }
            .toMap()

        val timestamp = parts["t"]?.toLongOrNull() ?: return false
        val provided = parts["v1"] ?: return false

        // Reject stale AND future-dated signatures. A far-future timestamp
        // would otherwise stay valid indefinitely, which is the replay
        // window this exists to close, reopened from the other side.
        val age = Duration.ofSeconds(clock.instant().epochSecond - timestamp).abs()
        if (age > tolerance) return false

        val expected = sign(timestamp, payload)
        return constantTimeEquals(expected, provided)
    }

    /**
     * The signature for a payload at a given time.
     *
     * Public so tests and a provider adapter can produce one without
     * reimplementing the scheme slightly differently — which is how the
     * two halves drift until verification silently rejects everything.
     */
    fun sign(timestamp: Long, payload: String): String {
        val mac = Mac.getInstance("HmacSHA256")
        mac.init(SecretKeySpec(secret.toByteArray(Charsets.UTF_8), "HmacSHA256"))
        // The timestamp is inside the signed material, not merely beside
        // it. Signing the body alone would let an attacker keep a valid
        // signature and simply update `t` to defeat the replay window.
        val signed = "$timestamp.$payload"
        return mac.doFinal(signed.toByteArray(Charsets.UTF_8))
            .joinToString("") { "%02x".format(it) }
    }

    /** Header a sender would attach right now. Used by tests and adapters. */
    fun header(payload: String): String {
        val now = clock.instant().epochSecond
        return "t=$now,v1=${sign(now, payload)}"
    }

    private fun constantTimeEquals(a: String, b: String): Boolean =
        MessageDigest.isEqual(
            a.toByteArray(Charsets.UTF_8),
            b.toByteArray(Charsets.UTF_8),
        )
}
