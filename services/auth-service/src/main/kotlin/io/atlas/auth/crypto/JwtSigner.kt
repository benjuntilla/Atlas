package io.atlas.auth.crypto

import io.atlas.auth.core.AuthError
import io.atlas.auth.core.TokenClaims
import org.jose4j.jws.AlgorithmIdentifiers
import org.jose4j.jws.JsonWebSignature
import org.jose4j.jwt.JwtClaims
import org.jose4j.jwt.consumer.InvalidJwtException
import org.jose4j.jwt.consumer.JwtConsumerBuilder
import org.jose4j.keys.HmacKey
import java.time.Instant
import java.util.UUID

/**
 * Signs and verifies Atlas JWTs.
 *
 * Token shape:
 * ```
 * {
 *   "sub": "<user_id>",
 *   "iat": <unix>,
 *   "exp": <unix>,
 *   "atlas:session_id": "<session_id>",
 *   "atlas:last_lat": <double>,        // optional
 *   "atlas:last_lng": <double>         // optional
 * }
 * ```
 *
 * HS256 in development. In production this rotates to RS256 with GCP KMS as
 * noted in the spec; the [JwtSigner] interface stays the same so the rest of
 * the service does not change.
 */
interface JwtSigner {
    fun sign(claims: TokenClaims): String

    /**
     * Returns parsed claims if the token is well-formed, signed by us, and
     * unexpired. Throws [AuthError.TokenInvalid] otherwise. This method
     * deliberately does not consult the session store; revocation is a
     * separate check performed by callers.
     */
    fun verify(token: String): TokenClaims
}

class Jose4jJwtSigner(secret: String) : JwtSigner {
    init {
        require(secret.length >= 32) {
            "JWT secret must be at least 32 bytes for HS256; got ${secret.length}"
        }
    }

    private val key = HmacKey(secret.toByteArray(Charsets.UTF_8))

    override fun sign(claims: TokenClaims): String {
        val jwt = JwtClaims().apply {
            subject = claims.userId.toString()
            issuedAt = org.jose4j.jwt.NumericDate.fromSeconds(claims.issuedAt.epochSecond)
            expirationTime = org.jose4j.jwt.NumericDate.fromSeconds(claims.expiresAt.epochSecond)
            setClaim(CLAIM_SESSION_ID, claims.sessionId.toString())
            if (claims.lastLat != null) setClaim(CLAIM_LAST_LAT, claims.lastLat)
            if (claims.lastLng != null) setClaim(CLAIM_LAST_LNG, claims.lastLng)
        }
        val jws = JsonWebSignature().apply {
            payload = jwt.toJson()
            key = this@Jose4jJwtSigner.key
            algorithmHeaderValue = AlgorithmIdentifiers.HMAC_SHA256
        }
        return jws.compactSerialization
    }

    override fun verify(token: String): TokenClaims {
        val consumer = JwtConsumerBuilder()
            .setRequireExpirationTime()
            .setRequireSubject()
            .setVerificationKey(key)
            .setJwsAlgorithmConstraints(
                org.jose4j.jwa.AlgorithmConstraints(
                    org.jose4j.jwa.AlgorithmConstraints.ConstraintType.PERMIT,
                    AlgorithmIdentifiers.HMAC_SHA256,
                )
            )
            .setAllowedClockSkewInSeconds(30)
            .build()

        val claims = try {
            consumer.processToClaims(token)
        } catch (e: InvalidJwtException) {
            throw AuthError.TokenInvalid(e.message ?: "invalid token")
        }

        val userId = try {
            UUID.fromString(claims.subject)
        } catch (e: IllegalArgumentException) {
            throw AuthError.TokenInvalid("sub is not a UUID")
        }

        val sessionIdStr = claims.getClaimValue(CLAIM_SESSION_ID, String::class.java)
            ?: throw AuthError.TokenInvalid("missing $CLAIM_SESSION_ID claim")
        val sessionId = try {
            UUID.fromString(sessionIdStr)
        } catch (e: IllegalArgumentException) {
            throw AuthError.TokenInvalid("$CLAIM_SESSION_ID is not a UUID")
        }

        return TokenClaims(
            userId = userId,
            sessionId = sessionId,
            issuedAt = Instant.ofEpochSecond(claims.issuedAt.value),
            expiresAt = Instant.ofEpochSecond(claims.expirationTime.value),
            lastLat = claims.getClaimValue(CLAIM_LAST_LAT, java.lang.Double::class.java)?.toDouble(),
            lastLng = claims.getClaimValue(CLAIM_LAST_LNG, java.lang.Double::class.java)?.toDouble(),
        )
    }

    companion object {
        const val CLAIM_SESSION_ID = "atlas:session_id"
        const val CLAIM_LAST_LAT = "atlas:last_lat"
        const val CLAIM_LAST_LNG = "atlas:last_lng"
    }
}
