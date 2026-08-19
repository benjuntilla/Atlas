package io.atlas.auth.memory

import io.atlas.auth.core.Session
import io.atlas.auth.core.SessionRepository
import io.atlas.auth.core.User
import io.atlas.auth.core.UserRepository
import io.atlas.auth.core.VerificationToken
import io.atlas.auth.core.VerificationTokenRepository
import java.time.Clock
import java.time.Instant
import java.util.UUID
import java.util.concurrent.ConcurrentHashMap

/**
 * In-memory implementations used by unit tests. Thread-safe via
 * [ConcurrentHashMap]. Not for production use.
 */

class InMemoryUserRepository(private val clock: Clock = Clock.systemUTC()) : UserRepository {
    private val byId = ConcurrentHashMap<UUID, User>()

    /**
     * Keyed by (project, email) rather than email, mirroring the
     * `users_project_email_key` constraint. A test double that enforced a
     * weaker rule than the database would let a scoping bug pass here and
     * fail in production, which is the wrong way round.
     */
    private val byProjectEmail = ConcurrentHashMap<Pair<UUID, String>, UUID>()

    override fun findByEmail(projectId: UUID, email: String): User? =
        byProjectEmail[projectId to email]?.let { byId[it] }

    override fun findById(projectId: UUID, id: UUID): User? =
        byId[id]?.takeIf { it.projectId == projectId }

    override fun create(projectId: UUID, email: String, passwordHash: String): User {
        val id = UUID.randomUUID()
        val user = User(
            id = id,
            projectId = projectId,
            email = email,
            passwordHash = passwordHash,
            createdAt = clock.instant(),
        )
        // Two-phase to keep the email index consistent with the row map.
        if (byProjectEmail.putIfAbsent(projectId to email, id) != null) {
            throw IllegalStateException("email already exists: $email")
        }
        byId[id] = user
        return user
    }

    override fun updatePasswordHash(projectId: UUID, userId: UUID, passwordHash: String) {
        byId.computeIfPresent(userId) { _, u ->
            if (u.projectId == projectId) u.copy(passwordHash = passwordHash) else u
        }
    }

    // Keeps the first timestamp, matching the `IS NULL` guard on the real
    // UPDATE. A double whose idempotency differed from the database's
    // would hide exactly the bug it exists to catch.
    override fun markEmailVerified(projectId: UUID, userId: UUID, at: Instant) {
        byId.computeIfPresent(userId) { _, u ->
            if (u.projectId == projectId && u.emailVerifiedAt == null) {
                u.copy(emailVerifiedAt = at)
            } else {
                u
            }
        }
    }
}

class InMemorySessionRepository : SessionRepository {
    private val sessions = ConcurrentHashMap<UUID, Session>()

    override fun create(userId: UUID, issuedAt: Instant, expiresAt: Instant): Session {
        val id = UUID.randomUUID()
        val session = Session(
            id = id,
            userId = userId,
            issuedAt = issuedAt,
            expiresAt = expiresAt,
            revoked = false,
        )
        sessions[id] = session
        return session
    }

    override fun findById(id: UUID): Session? = sessions[id]

    override fun revoke(id: UUID) {
        sessions.compute(id) { _, existing -> existing?.copy(revoked = true) }
    }

    override fun revokeAllForUser(userId: UUID): Int {
        var revoked = 0
        sessions.replaceAll { _, s ->
            if (s.userId == userId && !s.revoked) {
                revoked++
                s.copy(revoked = true)
            } else {
                s
            }
        }
        return revoked
    }
}

/**
 * In-memory single-use tokens.
 *
 * [consume] is synchronized and checks-then-sets under that lock, which is
 * the same guarantee the real `UPDATE ... WHERE used_at IS NULL` gives:
 * of two concurrent redemptions exactly one wins.
 */
class InMemoryVerificationTokenRepository : VerificationTokenRepository {
    private val byId = ConcurrentHashMap<UUID, VerificationToken>()
    private val byHash = ConcurrentHashMap<String, UUID>()

    override fun create(
        projectId: UUID,
        userId: UUID,
        purpose: String,
        tokenHash: String,
        expiresAt: Instant,
    ): VerificationToken {
        val token = VerificationToken(
            id = UUID.randomUUID(),
            projectId = projectId,
            userId = userId,
            purpose = purpose,
            expiresAt = expiresAt,
            usedAt = null,
        )
        byId[token.id] = token
        byHash[tokenHash] = token.id
        return token
    }

    override fun findByHash(tokenHash: String): VerificationToken? =
        byHash[tokenHash]?.let { byId[it] }

    @Synchronized
    override fun consume(id: UUID, usedAt: Instant): Boolean {
        val existing = byId[id] ?: return false
        if (existing.usedAt != null) return false
        byId[id] = existing.copy(usedAt = usedAt)
        return true
    }

    override fun invalidateAll(projectId: UUID, userId: UUID, purpose: String, at: Instant) {
        byId.replaceAll { _, t ->
            if (t.projectId == projectId &&
                t.userId == userId &&
                t.purpose == purpose &&
                t.usedAt == null
            ) {
                t.copy(usedAt = at)
            } else {
                t
            }
        }
    }
}
