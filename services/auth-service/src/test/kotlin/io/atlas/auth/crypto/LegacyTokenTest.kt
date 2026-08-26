package io.atlas.auth.crypto

import io.atlas.auth.core.TokenClaims
import org.junit.jupiter.api.Test
import java.time.Instant
import java.util.UUID
import kotlin.test.assertEquals

/**
 * Tokens minted before rotation support carry no `kid` at all.
 *
 * Deploying this change must not log those users out, so a token with no
 * kid is verified against the active key — the only key that could have
 * signed it, since retired keys are only ever introduced alongside a kid.
 */
class LegacyTokenTest {
    @Test
    fun `a token with no kid header still verifies`() {
        val secret = "the-first-secret-is-long-enough-yes!!"
        val userId = UUID.randomUUID()
        val now = Instant.now()

        // Hand-built without a kid, exactly as the old signer produced.
        val jwt = org.jose4j.jwt.JwtClaims().apply {
            subject = userId.toString()
            issuedAt = org.jose4j.jwt.NumericDate.fromSeconds(now.epochSecond)
            expirationTime = org.jose4j.jwt.NumericDate.fromSeconds(now.epochSecond + 3600)
            setClaim(Jose4jJwtSigner.CLAIM_PROJECT_ID, UUID.randomUUID().toString())
            setClaim(Jose4jJwtSigner.CLAIM_SESSION_ID, UUID.randomUUID().toString())
        }
        val jws = org.jose4j.jws.JsonWebSignature().apply {
            payload = jwt.toJson()
            key = org.jose4j.keys.HmacKey(secret.toByteArray(Charsets.UTF_8))
            algorithmHeaderValue = org.jose4j.jws.AlgorithmIdentifiers.HMAC_SHA256
            // No keyIdHeaderValue.
        }

        val signer = Jose4jJwtSigner(
            active = SigningKey("k1", secret),
            retired = listOf(SigningKey("k0", "an-older-secret-also-long-enough!!!!")),
        )
        assertEquals(userId, signer.verify(jws.compactSerialization).userId)
    }
}
