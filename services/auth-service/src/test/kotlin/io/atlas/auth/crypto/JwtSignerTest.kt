package io.atlas.auth.crypto

import io.atlas.auth.core.AuthError
import io.atlas.auth.core.TokenClaims
import org.junit.jupiter.api.Test
import java.time.Instant
import java.util.UUID
import kotlin.test.assertEquals
import kotlin.test.assertFailsWith
import kotlin.test.assertNull

class JwtSignerTest {

    private val secret = "this-secret-is-long-enough-for-hs256-yes"
    private val signer = Jose4jJwtSigner(secret)

    private fun claimsFixture(
        issuedAt: Instant = Instant.now(),
        expiresAt: Instant = Instant.now().plusSeconds(3600),
        lastLat: Double? = 33.4484,
        lastLng: Double? = -112.0740,
    ) = TokenClaims(
        userId = UUID.randomUUID(),
        sessionId = UUID.randomUUID(),
        issuedAt = issuedAt,
        expiresAt = expiresAt,
        lastLat = lastLat,
        lastLng = lastLng,
    )

    @Test
    fun `sign then verify round-trips claims`() {
        val original = claimsFixture()
        val token = signer.sign(original)
        val recovered = signer.verify(token)

        assertEquals(original.userId, recovered.userId)
        assertEquals(original.sessionId, recovered.sessionId)
        // JWT timestamps are second-precision; compare epoch seconds.
        assertEquals(original.issuedAt.epochSecond, recovered.issuedAt.epochSecond)
        assertEquals(original.expiresAt.epochSecond, recovered.expiresAt.epochSecond)
        assertEquals(original.lastLat, recovered.lastLat)
        assertEquals(original.lastLng, recovered.lastLng)
    }

    @Test
    fun `verify omits geo claims when absent`() {
        val original = claimsFixture(lastLat = null, lastLng = null)
        val recovered = signer.verify(signer.sign(original))
        assertNull(recovered.lastLat)
        assertNull(recovered.lastLng)
    }

    @Test
    fun `verify rejects token signed with a different secret`() {
        val other = Jose4jJwtSigner("a-totally-different-secret-that-is-long")
        val token = other.sign(claimsFixture())
        assertFailsWith<AuthError.TokenInvalid> { signer.verify(token) }
    }

    @Test
    fun `verify rejects tampered payload`() {
        val token = signer.sign(claimsFixture())
        // Flip a byte in the payload segment (between the two dots).
        val parts = token.split('.')
        val tampered = parts[0] + "." + parts[1].replaceRange(0, 1, "x") + "." + parts[2]
        assertFailsWith<AuthError.TokenInvalid> { signer.verify(tampered) }
    }

    @Test
    fun `verify rejects expired token`() {
        val past = Instant.now().minusSeconds(3600)
        val expired = claimsFixture(issuedAt = past.minusSeconds(60), expiresAt = past)
        val token = signer.sign(expired)
        assertFailsWith<AuthError.TokenInvalid> { signer.verify(token) }
    }

    @Test
    fun `verify rejects garbage`() {
        assertFailsWith<AuthError.TokenInvalid> { signer.verify("not a jwt") }
        assertFailsWith<AuthError.TokenInvalid> { signer.verify("") }
    }
}
