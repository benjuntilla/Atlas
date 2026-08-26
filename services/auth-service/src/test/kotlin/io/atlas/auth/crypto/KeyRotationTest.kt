package io.atlas.auth.crypto

import io.atlas.auth.config.EnvConfig
import io.atlas.auth.core.AuthError
import io.atlas.auth.core.TokenClaims
import org.junit.jupiter.api.Test
import java.time.Instant
import java.util.UUID
import kotlin.test.assertEquals
import kotlin.test.assertFailsWith
import kotlin.test.assertTrue

/**
 * JWT signing key rotation.
 *
 * These walk the three steps of a real rotation, because the property
 * that matters is not "two keys work" — it is that no step of the
 * sequence logs anybody out.
 */
class KeyRotationTest {

    private val oldKey = SigningKey("k1", "the-first-secret-is-long-enough-yes!!")
    private val newKey = SigningKey("k2", "the-second-secret-is-also-long-enough")

    private fun claims() = TokenClaims(
        userId = UUID.randomUUID(),
        projectId = UUID.randomUUID(),
        sessionId = UUID.randomUUID(),
        issuedAt = Instant.now(),
        expiresAt = Instant.now().plusSeconds(3600),
        lastLat = null,
        lastLng = null,
    )

    /** Step 0: one key, the state before a rotation begins. */
    @Test
    fun `a single key signs and verifies`() {
        val signer = Jose4jJwtSigner(oldKey)
        val c = claims()
        assertEquals(c.userId, signer.verify(signer.sign(c)).userId)
    }

    /**
     * Step 1: the new key is deployed as retired. Nothing signs with it
     * yet; every replica can verify it. Tokens are unaffected.
     */
    @Test
    fun `adding a retired key does not disturb existing tokens`() {
        val before = Jose4jJwtSigner(oldKey)
        val token = before.sign(claims())

        val during = Jose4jJwtSigner(active = oldKey, retired = listOf(newKey))
        assertEquals(
            before.verify(token).sessionId,
            during.verify(token).sessionId,
            "a token signed before the deploy must still verify after it",
        )
    }

    /**
     * Step 2: the new key becomes active and the old one is retired. This
     * is the step a single-key signer cannot survive — every token minted
     * before the switch would stop verifying at once.
     */
    @Test
    fun `promoting a key keeps tokens signed with the previous one valid`() {
        val beforeRotation = Jose4jJwtSigner(oldKey)
        val oldToken = beforeRotation.sign(claims())

        val afterRotation = Jose4jJwtSigner(active = newKey, retired = listOf(oldKey))

        // The old token still works...
        assertEquals(
            beforeRotation.verify(oldToken).userId,
            afterRotation.verify(oldToken).userId,
        )
        // ...and new tokens are signed with the new key.
        val newToken = afterRotation.sign(claims())
        assertEquals("k2", kidOf(newToken))
        assertEquals("k1", kidOf(oldToken))
    }

    /**
     * Step 3: the old key is dropped once every token signed with it has
     * expired. Only now does it stop verifying — which is the point of
     * waiting a token lifetime.
     */
    @Test
    fun `dropping a retired key finally invalidates its tokens`() {
        val oldToken = Jose4jJwtSigner(oldKey).sign(claims())
        val afterDrop = Jose4jJwtSigner(newKey)

        val e = assertFailsWith<AuthError.TokenInvalid> { afterDrop.verify(oldToken) }
        assertTrue(e.message!!.contains("unknown signing key"))
    }

    /**
     * A token naming a key we do not hold is rejected on the kid alone,
     * without attempting a signature check against every key we do hold.
     */
    @Test
    fun `a token naming an unknown key is refused`() {
        val stranger = Jose4jJwtSigner(SigningKey("k99", "a-key-this-deployment-never-had!!"))
        val token = stranger.sign(claims())

        assertFailsWith<AuthError.TokenInvalid> {
            Jose4jJwtSigner(active = newKey, retired = listOf(oldKey)).verify(token)
        }
    }

    /**
     * Holding the right kid is not enough — the signature still has to
     * check out. Guards against a `kid` lookup that accidentally became
     * the whole verification.
     */
    @Test
    fun `a token with a known kid but the wrong signature is refused`() {
        val impostor = Jose4jJwtSigner(SigningKey("k2", "a-different-secret-of-the-same-id!!!"))
        val forged = impostor.sign(claims())

        assertFailsWith<AuthError.TokenInvalid> {
            Jose4jJwtSigner(active = newKey, retired = listOf(oldKey)).verify(forged)
        }
    }

    /** Duplicate ids make verification ambiguous, so they are refused. */
    @Test
    fun `repeated key ids are rejected at construction`() {
        assertFailsWith<IllegalArgumentException> {
            Jose4jJwtSigner(
                active = SigningKey("k1", "the-first-secret-is-long-enough-yes!!"),
                retired = listOf(SigningKey("k1", "a-different-secret-same-identifier!!")),
            )
        }
    }

    @Test
    fun `a short secret is refused`() {
        assertFailsWith<IllegalArgumentException> { SigningKey("k1", "too-short") }
    }

    // --- config parsing ------------------------------------------------------

    @Test
    fun `retired keys parse from the env format`() {
        val parsed = EnvConfig.parseRetiredKeys("k1:secret-one,k0:secret-zero")
        assertEquals(listOf("k1" to "secret-one", "k0" to "secret-zero"), parsed)
        assertEquals(emptyList(), EnvConfig.parseRetiredKeys(null))
        assertEquals(emptyList(), EnvConfig.parseRetiredKeys("  "))
    }

    /**
     * A malformed entry throws rather than being skipped. Silently
     * dropping a retired key logs out every user holding a token signed
     * with it, and the only symptom is a support ticket.
     */
    @Test
    fun `a malformed retired key entry fails loudly`() {
        assertFailsWith<IllegalArgumentException> { EnvConfig.parseRetiredKeys("no-colon-here") }
        assertFailsWith<IllegalArgumentException> { EnvConfig.parseRetiredKeys(":no-id") }
        assertFailsWith<IllegalArgumentException> { EnvConfig.parseRetiredKeys("no-secret:") }
    }

    /** The third segment is the signature; the first is the header. */
    private fun kidOf(token: String): String? {
        val header = String(
            java.util.Base64.getUrlDecoder().decode(token.substringBefore('.')),
            Charsets.UTF_8,
        )
        return Regex("\"kid\"\\s*:\\s*\"([^\"]+)\"").find(header)?.groupValues?.get(1)
    }
}
