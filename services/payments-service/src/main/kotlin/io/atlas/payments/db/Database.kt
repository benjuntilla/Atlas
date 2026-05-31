package io.atlas.payments.db

import com.zaxxer.hikari.HikariConfig
import com.zaxxer.hikari.HikariDataSource
import org.jetbrains.exposed.sql.Database

/**
 * Hikari + Exposed bootstrap. Called once at startup by [io.atlas.payments.App].
 * `isAutoCommit = false` lets Exposed own transaction boundaries, which the
 * outbox pattern relies on (wallet mutation + outbox write commit together).
 */
object DatabaseBootstrap {
    fun connect(
        jdbcUrl: String,
        username: String,
        password: String,
        maximumPoolSize: Int = 10,
    ): Database {
        val config = HikariConfig().apply {
            this.jdbcUrl = jdbcUrl
            this.username = username
            this.password = password
            this.maximumPoolSize = maximumPoolSize
            this.driverClassName = "org.postgresql.Driver"
            this.isAutoCommit = false
            this.poolName = "atlas-payments-pool"
        }
        val dataSource = HikariDataSource(config)
        return Database.connect(dataSource)
    }
}
