package io.atlas.auth.db

import io.atlas.auth.core.Session
import io.atlas.auth.core.SessionRepository
import io.atlas.auth.core.User
import io.atlas.auth.core.UserRepository
import org.jetbrains.exposed.exceptions.ExposedSQLException
import org.jetbrains.exposed.sql.ResultRow
import org.jetbrains.exposed.sql.and
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
    projectId = this[Users.projectId],
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
    override fun findByEmail(projectId: UUID, email: String): User? = transaction {
        Users.selectAll()
            .where { (Users.projectId eq projectId) and (Users.email eq email) }
            .singleOrNull()?.toUser()
    }

    // Scoped even though `id` is a primary key and therefore already
    // unique. The point is not uniqueness, it is that looking up a user id
    // belonging to another tenant must return nothing rather than that
    // tenant's user.
    override fun findById(projectId: UUID, id: UUID): User? = transaction {
        Users.selectAll()
            .where { (Users.projectId eq projectId) and (Users.id eq id) }
            .singleOrNull()?.toUser()
    }

    override fun create(projectId: UUID, email: String, passwordHash: String): User = transaction {
        try {
            val id = Users.insert {
                it[Users.projectId] = projectId
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
