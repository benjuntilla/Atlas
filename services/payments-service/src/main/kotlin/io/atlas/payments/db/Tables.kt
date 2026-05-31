package io.atlas.payments.db

import org.jetbrains.exposed.sql.Table
import org.jetbrains.exposed.sql.javatime.timestamp

/**
 * Exposed table definitions matching migrations/0030_payments.sql and
 * 0031_payments_outbox.sql. The `payments.` schema prefix is built into each
 * table name so a single connection addresses every Atlas schema without a
 * `SET search_path` dance.
 *
 * Columns with a Postgres DEFAULT (id, created_at, balance_cents, status,
 * attempts, topic) are omitted from inserts and filled by the database.
 */

object Wallets : Table("payments.wallets") {
    val id = uuid("id").autoGenerate()
    val userId = uuid("user_id").uniqueIndex()
    val balanceCents = long("balance_cents")
    val currency = text("currency")
    val updatedAt = timestamp("updated_at")

    override val primaryKey = PrimaryKey(id)
}

object Transactions : Table("payments.transactions") {
    val id = uuid("id").autoGenerate()
    val fromWallet = uuid("from_wallet").nullable()
    val toWallet = uuid("to_wallet").nullable()
    val amountCents = long("amount_cents")
    val status = text("status")
    val idempotencyKey = text("idempotency_key").uniqueIndex()
    val rideId = uuid("ride_id").nullable()
    val createdAt = timestamp("created_at")
    val settledAt = timestamp("settled_at").nullable()
    val providerRef = text("provider_ref").nullable()
    val idempotencyArgsHash = text("idempotency_args_hash").nullable()

    override val primaryKey = PrimaryKey(id)
}

object Outbox : Table("payments.outbox") {
    val id = uuid("id").autoGenerate()
    val transactionId = uuid("transaction_id").nullable()
    val topic = text("topic")
    val payload = binary("payload")
    val createdAt = timestamp("created_at")
    val dispatchedAt = timestamp("dispatched_at").nullable()
    val attempts = integer("attempts")
    val lastError = text("last_error").nullable()

    override val primaryKey = PrimaryKey(id)
}
