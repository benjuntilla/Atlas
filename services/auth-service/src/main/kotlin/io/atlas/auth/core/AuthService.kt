package io.atlas.auth.core

import io.atlas.auth.crypto.JwtSigner
import io.atlas.auth.crypto.PasswordHasher
import java.time.Clock
import java.time.Duration
import java.util.UUID

/**
 * Transport-agnostic auth business logic. Phase 2B will wrap this in a gRPC
 * service. Tests inject in-memory repositories so the whole flow runs
 * without a database.
 *
 * Threading: instances are safe to share. The injected dependencies must
 * also be thread-safe (the production [PasswordHasher] and [JwtSigner] are;
 * the production Exposed-backed repositories rely on Exposed's transaction
 * semantics).
 */
class AuthService(
    private val users: UserRepository,
    private val sessions: SessionRepository,
    private val hasher: PasswordHasher,
    private val signer: JwtSigner,
    private val tokenLifetime: Duration = Duration.ofHours(1),
    private val clock: Clock = Clock.systemUTC(),
) {
    /**
     * Creates a new user. Throws [AuthError.EmailAlreadyExists] if the email
     * is already taken, [AuthError.InvalidEmail] or [AuthError.WeakPassword]
     * for malformed input.
     */
    fun register(email: String, password: String): UUID {
        val normalized = normalizeEmail(email)
        validateEmail(normalized)
        validatePassword(password)
        if (users.findByEmail(normalized) != null) {
            throw AuthError.EmailAlreadyExists(normalized)
        }
        val hash = hasher.hash(password)
        return users.create(email = normalized, passwordHash = hash).id
    }

    /**
     * Verifies credentials and issues a signed token bound to a fresh
     * session. Throws [AuthError.InvalidCredentials] on any failure so the
     * API does not leak whether the email is registered.
     */
    fun authenticate(
        email: String,
        password: String,
        lastLat: Double? = null,
        lastLng: Double? = null,
    ): SignedToken {
        val normalized = normalizeEmail(email)
        val user = users.findByEmail(normalized) ?: throw AuthError.InvalidCredentials()
        if (!hasher.verify(password, user.passwordHash)) {
            throw AuthError.InvalidCredentials()
        }
        val now = clock.instant()
        val expiresAt = now.plus(tokenLifetime)
        val session = sessions.create(userId = user.id, issuedAt = now, expiresAt = expiresAt)
        val claims = TokenClaims(
            userId = user.id,
            sessionId = session.id,
            issuedAt = now,
            expiresAt = expiresAt,
            lastLat = lastLat,
            lastLng = lastLng,
        )
        return SignedToken(token = signer.sign(claims), expiresAt = expiresAt)
    }

    /**
     * Refresh / re-issue primitive used by the internal `IssueToken` gRPC
     * call. The caller has already proven identity through some other
     * channel (typically a valid existing JWT) and is asking for a fresh
     * token tied to a new session.
     */
    fun issueTokenForUser(
        userId: UUID,
        lastLat: Double? = null,
        lastLng: Double? = null,
    ): SignedToken {
        val user = users.findById(userId) ?: throw AuthError.InvalidCredentials()
        val now = clock.instant()
        val expiresAt = now.plus(tokenLifetime)
        val session = sessions.create(userId = user.id, issuedAt = now, expiresAt = expiresAt)
        val claims = TokenClaims(
            userId = user.id,
            sessionId = session.id,
            issuedAt = now,
            expiresAt = expiresAt,
            lastLat = lastLat,
            lastLng = lastLng,
        )
        return SignedToken(token = signer.sign(claims), expiresAt = expiresAt)
    }

    // --- helpers ----------------------------------------------------------

    private fun normalizeEmail(email: String): String = email.trim().lowercase()

    private fun validateEmail(email: String) {
        // Intentionally permissive. RFC 5322 is unfeasibly large to validate
        // exactly; we just want to catch obvious typos and reject things
        // that clearly are not addresses.
        if (email.isEmpty()) throw AuthError.InvalidEmail(email)
        if (email.length > 254) throw AuthError.InvalidEmail(email)
        val at = email.indexOf('@')
        if (at <= 0 || at == email.length - 1) throw AuthError.InvalidEmail(email)
        val domain = email.substring(at + 1)
        if (!domain.contains('.')) throw AuthError.InvalidEmail(email)
        if (email.any { it.isWhitespace() }) throw AuthError.InvalidEmail(email)
    }

    private fun validatePassword(password: String) {
        if (password.length < MIN_PASSWORD_LENGTH) {
            throw AuthError.WeakPassword("must be at least $MIN_PASSWORD_LENGTH characters")
        }
        if (password.length > MAX_PASSWORD_LENGTH) {
            // bcrypt truncates at 72 bytes. Refusing longer inputs is more
            // honest than silently ignoring the tail.
            throw AuthError.WeakPassword("must be at most $MAX_PASSWORD_LENGTH characters")
        }
        if (password.isBlank()) {
            throw AuthError.WeakPassword("must contain non-whitespace characters")
        }
    }

    companion object {
        const val MIN_PASSWORD_LENGTH = 8
        const val MAX_PASSWORD_LENGTH = 72
    }
}
