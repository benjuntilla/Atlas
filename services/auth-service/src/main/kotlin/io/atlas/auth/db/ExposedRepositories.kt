package io.atlas.auth.db

import io.atlas.auth.core.Session
import io.atlas.auth.core.SessionRepository
import io.atlas.auth.core.User
import io.atlas.auth.core.UserRepository
import org.jetbrains.exposed.exceptions.ExposedSQLException
import org.jetbrains.exposed.sql.ResultRow
import org.jetbrains.exposed.sql.insert
import org.jetbrains.exposed.sql.select
import org.jetbrains.exposed.sql.selectAll
import org.jetbrains.exposed.sql.transactions.transaction
import org.jetbrains.exposed.sql.update
import java.time.Instant
import java.util.UUID

/**
 * Postgres-backed repositories. Each method opens its own Exposed
 * transaction. Composing higher-level transactions across multiple methods
 * is not currently needed; if it becomes needed (e.g. atomic create-user-
 * and-issue-session) the AuthService should wrap the calls in a single
 * `transaction { ... }` block.
 */

private fun ResultRow.toUser() = User(
    id = this[Users.id],
    email = this[Users.email],
    passwordHash = this[Users.passwordHash],
    createdAt = this[Users.createdAt],
)

private fun ResultRow.toSession() = Session(
    id = this[Sessions.id],
    userId = this[Sessions.userId],
    issuedAt = this[Sessions.issuedAt],
    expiresAt = this[Sessions.expiresAt],
    revoked = this[Sessions.revoked],
)

class ExposedUserRepository : UserRepository {
    override fun findByEmail(email: String): User? = transaction {
        Users.selectAll().where { Users.email eq email }.singleOrNull()?.toUser()
    }

    override fun findById(id: UUID): User? = transaction {
        Users.selectAll().where { Users.id eq id }.singleOrNull()?.toUser()
    }

    override fun create(email: String, passwordHash: String): User = transaction {
        try {
            val id = Users.insert {
                it[Users.email] = email
                it[Users.passwordHash] = passwordHash
                it[Users.createdAt] = Instant.now()
            } get Users.id
            Users.selectAll().where { Users.id eq id }.single().toUser()
        } catch (e: ExposedSQLException) {
            // Postgres unique-violation maps to SQLSTATE 23505. Surfacing it as
            // the same IllegalStateException the in-memory impl uses keeps the
            // AuthService unaware of the persistence layer.
            if (e.sqlState == "23505") {
                throw IllegalStateException("email already exists: $email", e)
            }
            throw e
        }
    }
}

class ExposedSessionRepository : SessionRepository {
    override fun create(userId: UUID, issuedAt: Instant, expiresAt: Instant): Session = transaction {
        val id = Sessions.insert {
            it[Sessions.userId] = userId
            it[Sessions.issuedAt] = issuedAt
            it[Sessions.expiresAt] = expiresAt
        } get Sessions.id
        Sessions.selectAll().where { Sessions.id eq id }.single().toSession()
    }

    override fun findById(id: UUID): Session? = transaction {
        Sessions.selectAll().where { Sessions.id eq id }.singleOrNull()?.toSession()
    }

    override fun revoke(id: UUID) {
        transaction {
            Sessions.update({ Sessions.id eq id }) {
                it[revoked] = true
            }
        }
    }
}
