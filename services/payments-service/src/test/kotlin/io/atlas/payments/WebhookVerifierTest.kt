package io.atlas.payments

import io.atlas.payments.core.HmacWebhookVerifier
import kotlin.test.Test
import kotlin.test.assertFalse
import kotlin.test.assertTrue
import kotlin.test.assertFailsWith
import java.time.Clock
import java.time.Duration
import java.time.Instant
import java.time.ZoneOffset

/**
 * Webhook signature verification.
 *
 * This endpoint is an unauthenticated write path from the public internet
 * into a payments system. The signature is the only thing making it safe,
 * so these tests are mostly about what it REFUSES.
 */
class WebhookVerifierTest {

    private val secret = "whsec_a_reasonably_long_shared_secret"
    private val now = Instant.parse("2026-08-19T12:00:00Z")
    private val clock = Clock.fixed(now, ZoneOffset.UTC)

    private fun verifier(tolerance: Duration = Duration.ofMinutes(5)) =
        HmacWebhookVerifier(secret, tolerance, clock)

    private val payload = """{"id":"evt_1","type":"charge.captured"}"""

    @Test
    fun `a correctly signed payload verifies`() {
        val v = verifier()
        assertTrue(v.verify(payload, v.header(payload)))
    }

    @Test
    fun `a tampered payload does not verify`() {
        val v = verifier()
        val header = v.header(payload)
        // One character changed: an amount, an id, anything.
        assertFalse(v.verify(payload.replace("evt_1", "evt_2"), header))
    }

    @Test
    fun `a signature from a different secret does not verify`() {
        val attacker = HmacWebhookVerifier("whsec_some_other_secret_entirely", clock = clock)
        assertFalse(verifier().verify(payload, attacker.header(payload)))
    }

    // --- replay ---------------------------------------------------------

    /**
     * Without a timestamp window a captured signature is valid forever,
     * so anyone who observes one legitimate delivery can replay it — at a
     * payments system, repeatedly.
     */
    @Test
    fun `a signature older than the tolerance is refused`() {
        val v = verifier(Duration.ofMinutes(5))
        val old = now.minus(Duration.ofMinutes(10)).epochSecond
        val header = "t=$old,v1=${v.sign(old, payload)}"
        assertFalse(v.verify(payload, header), "a ten-minute-old signature must not verify")
    }

    /** And the boundary is honoured rather than being off by one. */
    @Test
    fun `a signature just inside the tolerance is accepted`() {
        val v = verifier(Duration.ofMinutes(5))
        val recent = now.minus(Duration.ofMinutes(4)).epochSecond
        assertTrue(v.verify(payload, "t=$recent,v1=${v.sign(recent, payload)}"))
    }

    /**
     * Future-dated too. A signature stamped a year ahead would otherwise
     * stay valid for a year — the same replay window, reopened from the
     * other side.
     */
    @Test
    fun `a far-future signature is refused`() {
        val v = verifier(Duration.ofMinutes(5))
        val future = now.plus(Duration.ofDays(365)).epochSecond
        assertFalse(v.verify(payload, "t=$future,v1=${v.sign(future, payload)}"))
    }

    /**
     * The timestamp is part of the signed material, so an attacker cannot
     * keep a valid signature and simply move `t` forward to escape the
     * replay window.
     */
    @Test
    fun `moving the timestamp invalidates the signature`() {
        val v = verifier()
        val old = now.minus(Duration.ofHours(2)).epochSecond
        val stolen = v.sign(old, payload)
        // Same signature, fresh timestamp.
        assertFalse(v.verify(payload, "t=${now.epochSecond},v1=$stolen"))
    }

    // --- malformed input -------------------------------------------------

    @Test
    fun `missing or malformed headers are refused`() {
        val v = verifier()
        assertFalse(v.verify(payload, null))
        assertFalse(v.verify(payload, ""))
        assertFalse(v.verify(payload, "garbage"))
        assertFalse(v.verify(payload, "t=notanumber,v1=abc"))
        // A signature with no timestamp cannot be replay-checked at all.
        assertFalse(v.verify(payload, "v1=${v.sign(now.epochSecond, payload)}"))
        // A timestamp with no signature proves nothing.
        assertFalse(v.verify(payload, "t=${now.epochSecond}"))
    }

    @Test
    fun `an empty secret is refused at construction`() {
        // An empty secret verifies nothing while looking like it works.
        assertFailsWith<IllegalArgumentException> { HmacWebhookVerifier("") }
        assertFailsWith<IllegalArgumentException> { HmacWebhookVerifier("   ") }
    }

    /**
     * Extra fields are tolerated: providers add them over time, and a
     * verifier that broke on an unrecognised key would fail closed on a
     * routine provider change.
     */
    @Test
    fun `unknown header fields are ignored`() {
        val v = verifier()
        val t = now.epochSecond
        val header = "t=$t,v1=${v.sign(t, payload)},v0=legacy,foo=bar"
        assertTrue(v.verify(payload, header))
    }

    /**
     * A signature is over exact bytes. Re-serialising JSON changes key
     * order and whitespace, so a verifier fed anything but the raw body
     * rejects legitimate traffic — the classic way this integration
     * breaks.
     */
    @Test
    fun `whitespace differences break the signature, as they must`() {
        val v = verifier()
        val header = v.header(payload)
        val reserialised = """{ "id": "evt_1", "type": "charge.captured" }"""
        assertFalse(v.verify(reserialised, header))
    }
}
