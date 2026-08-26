package io.atlas.fare.db

import org.jetbrains.exposed.sql.Column
import org.jetbrains.exposed.sql.ColumnType
import org.jetbrains.exposed.sql.Table
import org.jetbrains.exposed.sql.javatime.timestamp
import org.postgresql.util.PGobject

/**
 * Minimal `jsonb` column type.
 *
 * `payments.transaction_events.payload` is JSONB. Binding a Kotlin String
 * to it through the normal `text()` column sends a VARCHAR parameter, and
 * Postgres refuses with "column payload is of type jsonb but expression
 * is of type character varying" — at runtime, on the first event, which
 * is a bad place to find out. Wrapping the value in a PGobject tagged
 * `jsonb` binds it correctly.
 *
 * The alternative, putting `stringtype=unspecified` in the JDBC URL,
 * would fix this by loosening type checking for every parameter on the
 * connection. Too blunt.
 */
private class JsonbColumnType : ColumnType<String>() {
    override fun sqlType(): String = "jsonb"

    override fun valueFromDB(value: Any): String = when (value) {
        is PGobject -> value.value.orEmpty()
        else -> value.toString()
    }

    override fun notNullValueToDB(value: String): Any = PGobject().apply {
        type = "jsonb"
        this.value = value
    }
}

private fun Table.jsonb(name: String): Column<String> = registerColumn(name, JsonbColumnType())

/**
 * The two payments tables this consumer touches.
 *
 * [Transactions] is READ-ONLY here — used solely to resolve a ride_id to
 * the transaction payments created for it. Every mutation goes through
 * payments-service over gRPC so its invariants (balance checks, provider
 * capture, outbox write, status transition) stay in one transaction owned
 * by one service.
 *
 * [TransactionEvents] is this consumer's own table. Migration 0030
 * created it "for fare-consumer" and 0032 added the event_key unique
 * index that makes redelivery a no-op.
 */

object Transactions : Table("payments.transactions") {
    val id = uuid("id")
    val projectId = uuid("project_id")
    val rideId = uuid("ride_id").nullable()
    val status = text("status")
    val createdAt = timestamp("created_at")

    override val primaryKey = PrimaryKey(id)
}

object TransactionEvents : Table("payments.transaction_events") {
    val id = uuid("id").autoGenerate()
    val projectId = uuid("project_id")
    val transactionId = uuid("transaction_id").nullable()
    val rideId = uuid("ride_id").nullable()
    val eventType = text("event_type")
    val payload = jsonb("payload")
    val eventKey = text("event_key")
    val createdAt = timestamp("created_at")

    override val primaryKey = PrimaryKey(id)

    // (project, key), matching `idx_transaction_events_project_key` from
    // migration 0050. event_key is a hash over fields the caller supplies
    // — ride_id among them — so two tenants can legitimately produce the
    // same one, and a global dedup index would drop the second tenant's
    // audit row on the floor.
    init {
        uniqueIndex(projectId, eventKey)
    }
}
