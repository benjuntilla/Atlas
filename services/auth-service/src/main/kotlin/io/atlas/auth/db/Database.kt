package io.atlas.auth.db

import com.zaxxer.hikari.HikariConfig
import com.zaxxer.hikari.HikariDataSource
import org.jetbrains.exposed.sql.Database

/**
 * Hikari + Exposed bootstrap. Called once at startup by the Phase 2B entry
 * point (`App.kt`). Kept tiny on purpose: configuration knobs that we have
 * actually needed in the past are exposed; everything else relies on
 * Hikari/Exposed defaults.
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
            this.isAutoCommit = false   // Exposed manages its own transactions
            this.poolName = "atlas-auth-pool"
        }
        val dataSource = HikariDataSource(config)
        return Database.connect(dataSource)
    }
}
