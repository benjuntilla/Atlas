package io.atlas.auth.core

import java.security.MessageDigest
import java.security.SecureRandom
import java.time.Instant
import java.util.UUID

/**
 * Single-use secrets mailed to a user: password reset and email
 * verification.
 *
 * # Why the hash and not the token
 *
 * Only the SHA-256 of a token is stored, the same rule API keys follow.
 * The plaintext exists in exactly one place — the email — so a database
 * dump, a backup, or a stray log line cannot be redeemed. SHA-256 rather
 * than bcrypt because unlike a password this is 256 bits of uniform
 * randomness: there is no dictionary to attack, so the slow hash would
 * buy nothing and cost a lot on a hot path.
 */
object VerificationTokens {
    /**
     * 32 bytes from [SecureRandom], hex-encoded.
     *
     * Long enough that guessing is not an attack anyone runs, and short
     * enough to survive an email client wrapping the link.
     */
    fun generate(): String {
        val bytes = ByteArray(32)
        RANDOM.nextBytes(bytes)
        return bytes.joinToString("") { "%02x".format(it) }
    }

    fun hash(token: String): String =
        MessageDigest.getInstance("SHA-256")
            .digest(token.toByteArray(Charsets.UTF_8))
            .joinToString("") { "%02x".format(it) }

    private val RANDOM = SecureRandom()
}

object TokenPurpose {
    const val PASSWORD_RESET = "password_reset"
    const val EMAIL_VERIFICATION = "email_verification"
}

/** A token row, minus the plaintext nobody can recover. */
data class VerificationToken(
    val id: UUID,
    val projectId: UUID,
    val userId: UUID,
    val purpose: String,
    val expiresAt: Instant,
    val usedAt: Instant?,
)

interface VerificationTokenRepository {
    /** Store a freshly minted token. [tokenHash] is the SHA-256 hex. */
    fun create(
        projectId: UUID,
        userId: UUID,
        purpose: String,
        tokenHash: String,
        expiresAt: Instant,
    ): VerificationToken

    /**
     * Look up by hash — deliberately NOT scoped by project.
     *
     * The token is the credential here, and it is 256 bits of randomness
     * that Atlas itself generated; the row it finds names its own project.
     * Scoping the lookup would mean asking the redeemer which project they
     * are in, and the redeemer is someone clicking a link in an email who
     * has no idea. The scoping instead happens on the row: the caller
     * checks the returned project against whatever it is acting on.
     */
    fun findByHash(tokenHash: String): VerificationToken?

    /**
     * Mark redeemed, but only if it is still unredeemed.
     *
     * Returns false if another request consumed it first. This is the
     * whole single-use guarantee, and it lives in one conditional UPDATE
     * rather than a read-then-write, because two clicks on the same link
     * arriving together must not both succeed.
     */
    fun consume(id: UUID, usedAt: Instant): Boolean

    /**
     * Invalidate every live token of one purpose for one user.
     *
     * Called when a reset succeeds: any other outstanding reset link must
     * stop working, or an attacker who requested one earlier keeps a way
     * in after the real user has recovered the account.
     */
    fun invalidateAll(projectId: UUID, userId: UUID, purpose: String, at: Instant)
}
