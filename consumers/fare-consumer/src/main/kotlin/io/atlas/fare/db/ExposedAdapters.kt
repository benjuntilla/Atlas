package io.atlas.fare.db

import com.zaxxer.hikari.HikariConfig
import com.zaxxer.hikari.HikariDataSource
import io.atlas.fare.core.AuditEntry
import io.atlas.fare.core.AuditLog
import io.atlas.fare.core.TransactionLookup
import org.jetbrains.exposed.sql.Database
import org.jetbrains.exposed.sql.SortOrder
import org.jetbrains.exposed.sql.selectAll
import org.jetbrains.exposed.sql.and
import org.jetbrains.exposed.sql.insertIgnore
import org.jetbrains.exposed.sql.transactions.transaction
import java.util.UUID

object DatabaseBootstrap {
    fun connect(
        jdbcUrl: String,
        username: String,
        password: String,
        maximumPoolSize: Int = 5,
    ): Database {
        val config = HikariConfig().apply {
            this.jdbcUrl = jdbcUrl
            this.username = username
            this.password = password
            this.maximumPoolSize = maximumPoolSize
            this.driverClassName = "org.postgresql.Driver"
            this.isAutoCommit = false
            this.poolName = "atlas-fare-consumer-pool"
        }
        return Database.connect(HikariDataSource(config))
    }
}

class ExposedTransactionLookup : TransactionLookup {
    /**
     * Find the transaction for a ride.
     *
     * A ride can in principle have more than one row — an initiate that
     * was refunded and retried under a fresh idempotency key — so this
     * takes the most recent, which is the one a completion or
     * cancellation refers to. Refunded and failed rows are skipped so a
     * dead earlier attempt cannot shadow the live one.
     */
    override fun findByRideId(projectId: UUID, rideId: UUID): UUID? = transaction {
        Transactions
            .selectAll()
            .where {
                // Scoped, and this one is not defensive padding: ride_id
                // is opaque to Atlas and chosen by the caller, so two
                // tenants using the same one is normal. Unscoped, this
                // could resolve a ride to ANOTHER tenant's transaction and
                // hand it to settle().
                (Transactions.projectId eq projectId) and
                    (Transactions.rideId eq rideId) and
                    (Transactions.status inList listOf("pending", "settled"))
            }
            .orderBy(Transactions.createdAt to SortOrder.DESC)
            .limit(1)
            .firstOrNull()
            ?.get(Transactions.id)
    }
}

class ExposedAuditLog : AuditLog {
    /**
     * Insert one audit row, ignoring a duplicate.
     *
     * `insertIgnore` compiles to `ON CONFLICT DO NOTHING`, which the
     * unique index on event_key (migration 0032) turns into replay
     * suppression. The returned row count distinguishes "recorded" from
     * "already knew about this".
     */
    override fun record(entry: AuditEntry): Boolean = transaction {
        val inserted = TransactionEvents.insertIgnore {
            it[projectId] = entry.projectId
            it[transactionId] = entry.transactionId
            it[rideId] = entry.rideId
            it[eventType] = entry.eventType.name
            it[payload] = entry.payloadJson
            it[eventKey] = entry.eventKey
        }
        inserted.insertedCount > 0
    }
}
