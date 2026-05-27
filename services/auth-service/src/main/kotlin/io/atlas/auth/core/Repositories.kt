package io.atlas.auth.core

import java.time.Instant
import java.util.UUID

/**
 * Persistence contracts. Production implementations live in [io.atlas.auth.db]
 * (Exposed + Postgres). Tests use the in-memory implementations in
 * [io.atlas.auth.memory] so the AuthService can be exercised without a DB.
 */

interface UserRepository {
    fun findByEmail(email: String): User?
    fun findById(id: UUID): User?
    /** Creates and returns the persisted [User]. Throws if [email] is not unique. */
    fun create(email: String, passwordHash: String): User
}

interface SessionRepository {
    fun create(userId: UUID, issuedAt: Instant, expiresAt: Instant): Session
    fun findById(id: UUID): Session?
    /** Marks the session revoked. No-op if already revoked or unknown. */
    fun revoke(id: UUID)
}
