package io.atlas.auth

import io.atlas.auth.cache.TokenValidationCache
import io.atlas.auth.config.EnvConfig
import io.atlas.auth.core.AuthService
import io.atlas.auth.crypto.BcryptPasswordHasher
import io.atlas.auth.crypto.Jose4jJwtSigner
import io.atlas.auth.db.DatabaseBootstrap
import io.atlas.auth.db.ExposedSessionRepository
import io.atlas.auth.db.ExposedUserRepository
import io.atlas.auth.grpc.AuthGrpcService
import io.atlas.auth.grpc.HealthCheck
import io.atlas.auth.kafka.AuthTokenProducer
import io.atlas.auth.kafka.TokenEventConsumer
import io.grpc.ServerBuilder
import org.slf4j.LoggerFactory
import java.time.Clock
import java.time.Duration
import java.util.concurrent.TimeUnit

/**
 * Phase 2B entry point. Reads config from env, opens the Postgres pool,
 * constructs the AuthService dependency graph, starts a gRPC server on
 * :50051 with grpc.health.v1.Health registered, fires up the Kafka producer
 * and the cache-invalidation consumer, and waits for the JVM to terminate.
 *
 * Graceful shutdown stops accepting new RPCs, flips health to NOT_SERVING,
 * drains in-flight requests, closes Kafka clients, and closes the DB pool.
 */
private val LOG = LoggerFactory.getLogger("io.atlas.auth.App")

fun main() {
    val config = EnvConfig.fromEnv()
    LOG.info(
        "starting auth-service: grpcPort={} dbUrl={} kafkaBrokers={}",
        config.grpcPort, config.databaseUrl, config.kafkaBrokers,
    )

    DatabaseBootstrap.connect(
        jdbcUrl = config.databaseUrl,
        username = config.databaseUser,
        password = config.databasePassword,
    )

    val users = ExposedUserRepository()
    val sessions = ExposedSessionRepository()
    val hasher = BcryptPasswordHasher(cost = 12)
    val signer = Jose4jJwtSigner(config.jwtSecret)
    val authService = AuthService(
        users = users,
        sessions = sessions,
        hasher = hasher,
        signer = signer,
        tokenLifetime = Duration.ofSeconds(config.tokenLifetimeSeconds),
        clock = Clock.systemUTC(),
    )

    val cache = TokenValidationCache()
    val producer = AuthTokenProducer.build(config.kafkaBrokers)
    val consumer = TokenEventConsumer(
        bootstrapServers = config.kafkaBrokers,
        cache = cache,
    )

    val grpcService = AuthGrpcService(
        authService = authService,
        sessions = sessions,
        signer = signer,
        cache = cache,
        publisher = producer,
    )

    val health = HealthCheck()

    val server = ServerBuilder.forPort(config.grpcPort)
        .addService(grpcService)
        .addService(health.service)
        .build()

    server.start()
    health.setServing()
    health.setServing("atlas.auth.AuthService")
    consumer.start()
    LOG.info("started gRPC server on :{}", config.grpcPort)

    Runtime.getRuntime().addShutdownHook(Thread {
        LOG.info("shutdown signal received, draining")
        health.setNotServing()
        health.shutdown()
        try {
            server.shutdown().awaitTermination(15, TimeUnit.SECONDS)
        } catch (e: InterruptedException) {
            Thread.currentThread().interrupt()
        }
        consumer.close()
        producer.close()
        LOG.info("auth-service stopped")
    })

    server.awaitTermination()
}
