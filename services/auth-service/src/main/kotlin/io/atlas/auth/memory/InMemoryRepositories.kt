package io.atlas.auth.memory

import io.atlas.auth.core.Session
import io.atlas.auth.core.SessionRepository
import io.atlas.auth.core.User
import io.atlas.auth.core.UserRepository
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
    private val byEmail = ConcurrentHashMap<String, UUID>()

    override fun findByEmail(email: String): User? = byEmail[email]?.let { byId[it] }

    override fun findById(id: UUID): User? = byId[id]

    override fun create(email: String, passwordHash: String): User {
        val id = UUID.randomUUID()
        val user = User(id = id, email = email, passwordHash = passwordHash, createdAt = clock.instant())
        // Two-phase to keep the email index consistent with the row map.
        if (byEmail.putIfAbsent(email, id) != null) {
            throw IllegalStateException("email already exists: $email")
        }
        byId[id] = user
        return user
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
}
