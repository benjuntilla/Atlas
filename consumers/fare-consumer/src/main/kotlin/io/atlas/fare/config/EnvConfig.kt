package io.atlas.fare.config

/**
 * Runtime configuration from the environment. Defaults match
 * docker-compose.yml so the consumer works in the local stack unmodified.
 */
data class EnvConfig(
    val databaseUrl: String,
    val databaseUser: String,
    val databasePassword: String,
    val kafkaBrokers: String,
    val fareTopic: String,
    val consumerGroup: String,
    val paymentsTarget: String,
    val httpPort: Int,
) {
    companion object {
        fun fromEnv(env: Map<String, String> = System.getenv()): EnvConfig {
            val raw = env["DATABASE_URL"] ?: "postgres://atlas:atlas_dev@localhost:5432/atlas"
            val (jdbcUrl, user, password) = parseDatabaseUrl(raw)
            return EnvConfig(
                databaseUrl = jdbcUrl,
                databaseUser = user,
                databasePassword = password,
                kafkaBrokers = env["KAFKA_BROKERS"] ?: "localhost:9092",
                fareTopic = env["KAFKA_TOPIC_FARE_EVENTS"] ?: "atlas.fare.events",
                consumerGroup = env["CONSUMER_GROUP"] ?: "atlas-fare-consumer",
                // host:port, not a URL — gRPC's ManagedChannelBuilder takes
                // a target, and passing "http://..." here makes it try to
                // resolve a nameserver called "http".
                paymentsTarget = env["PAYMENTS_SERVICE_TARGET"] ?: "localhost:50053",
                httpPort = env["HTTP_PORT"]?.toInt() ?: 8054,
            )
        }

        // Same libpq-to-JDBC translation as payments-service. Duplicated
        // rather than shared because the two modules have no common
        // library, and a shared one for eight lines is not worth the
        // coupling.
        private fun parseDatabaseUrl(raw: String): Triple<String, String, String> {
            if (raw.startsWith("jdbc:")) return Triple(raw, "", "")
            val withoutScheme = raw.removePrefix("postgres://").removePrefix("postgresql://")
            val atIdx = withoutScheme.lastIndexOf('@')
            require(atIdx > 0) { "DATABASE_URL must include credentials: $raw" }
            val credentials = withoutScheme.substring(0, atIdx)
            val hostAndPath = withoutScheme.substring(atIdx + 1)
            val colonIdx = credentials.indexOf(':')
            val user = if (colonIdx >= 0) credentials.substring(0, colonIdx) else credentials
            val password = if (colonIdx >= 0) credentials.substring(colonIdx + 1) else ""
            return Triple("jdbc:postgresql://$hostAndPath", user, password)
        }
    }
}
