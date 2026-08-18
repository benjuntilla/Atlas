package io.atlas.auth.core

import java.time.Instant
import java.util.UUID

/**
 * Persistence contracts. Production implementations live in [io.atlas.auth.db]
 * (Exposed + Postgres). Tests use the in-memory implementations in
 * [io.atlas.auth.memory] so the AuthService can be exercised without a DB.
 */

/**
 * Every lookup takes a projectId. It is the first parameter rather than an
 * optional filter so that a caller cannot write an unscoped query by
 * omission — the type system asks the question on every call.
 */
interface UserRepository {
    fun findByEmail(projectId: UUID, email: String): User?
    fun findById(projectId: UUID, id: UUID): User?
    /**
     * Creates and returns the persisted [User]. Throws if [email] is already
     * taken WITHIN [projectId]; the same address in another project is a
     * different person and is allowed.
     */
    fun create(projectId: UUID, email: String, passwordHash: String): User
}

interface SessionRepository {
    fun create(userId: UUID, issuedAt: Instant, expiresAt: Instant): Session
    fun findById(id: UUID): Session?
    /** Marks the session revoked. No-op if already revoked or unknown. */
    fun revoke(id: UUID)
}
