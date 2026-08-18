package io.atlas.auth.db

import org.jetbrains.exposed.sql.Table
import org.jetbrains.exposed.sql.javatime.timestamp

/**
 * Exposed table definitions matching `migrations/0010_auth.sql`. The
 * `auth.` schema prefix is built into each table name so a single connection
 * can address every Atlas schema without a `SET search_path` dance.
 */

object Users : Table("auth.users") {
    val id = uuid("id").autoGenerate()
    val projectId = uuid("project_id")
    val email = text("email")
    val passwordHash = text("password_hash")
    val createdAt = timestamp("created_at")

    override val primaryKey = PrimaryKey(id)

    // Matches `users_project_email_key` from migration 0050. The old
    // declaration marked `email` unique on its own, which is what the
    // schema used to say and is now wrong in the way that matters: it
    // would have one customer's signup fail because a different customer
    // already had that address.
    init {
        uniqueIndex(projectId, email)
    }
}

object Sessions : Table("auth.sessions") {
    val id = uuid("id").autoGenerate()
    val userId = uuid("user_id").references(Users.id)
    val issuedAt = timestamp("issued_at")
    val expiresAt = timestamp("expires_at")
    val revoked = bool("revoked").default(false)

    override val primaryKey = PrimaryKey(id)
}
