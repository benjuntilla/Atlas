package io.atlas.payments

import io.atlas.payments.config.EnvConfig
import io.atlas.payments.core.FakePaymentProvider
import io.atlas.payments.core.PaymentsService
import io.atlas.payments.db.DatabaseBootstrap
import io.atlas.payments.db.ExposedOutboxBackend
import io.atlas.payments.db.ExposedOutboxStore
import io.atlas.payments.db.ExposedTransactionRepository
import io.atlas.payments.db.ExposedTransactionRunner
import io.atlas.payments.db.ExposedWalletRepository
import io.atlas.payments.grpc.HealthCheck
import io.atlas.payments.grpc.PaymentsGrpcService
import io.atlas.payments.http.MicrometerPaymentsMetrics
import io.atlas.payments.http.newPrometheusRegistry
import io.atlas.payments.http.startHttpServer
import io.atlas.payments.kafka.FareEventProducer
import io.atlas.payments.outbox.OutboxDispatcher
import io.grpc.ServerBuilder
import org.slf4j.LoggerFactory
import java.time.Clock
import java.time.Duration
import java.util.concurrent.TimeUnit

/**
 * Phase 4 entry point. Reads config from env, opens the Postgres pool, builds
 * the PaymentsService graph, starts a gRPC server on :50053 (with
 * grpc.health.v1.Health), a Ktor HTTP server on :8053 (/metrics + webhooks),
 * and the background outbox dispatcher, then waits for the JVM to terminate.
 *
 * Graceful shutdown flips health to NOT_SERVING, stops the dispatcher, drains
 * in-flight RPCs, stops the HTTP server, and closes the Kafka producer.
 */
private val LOG = LoggerFactory.getLogger("io.atlas.payments.App")

fun main() {
    val config = EnvConfig.fromEnv()
    LOG.info(
        "starting payments-service: grpcPort={} httpPort={} dbUrl={} kafkaBrokers={} fareTopic={}",
        config.grpcPort, config.httpPort, config.databaseUrl, config.kafkaBrokers, config.fareTopic,
    )

    DatabaseBootstrap.connect(
        jdbcUrl = config.databaseUrl,
        username = config.databaseUser,
        password = config.databasePassword,
    )

    val registry = newPrometheusRegistry()
    val metrics = MicrometerPaymentsMetrics(registry)

    val wallets = ExposedWalletRepository()
    val transactions = ExposedTransactionRepository()
    val outboxStore = ExposedOutboxStore()
    val runner = ExposedTransactionRunner()
    val provider = FakePaymentProvider()

    val payments = PaymentsService(
        wallets = wallets,
        transactions = transactions,
        outbox = outboxStore,
        runner = runner,
        provider = provider,
        fareTopic = config.fareTopic,
        clock = Clock.systemUTC(),
        metrics = metrics,
    )

    val publisher = FareEventProducer.build(config.kafkaBrokers)
    val dispatcher = OutboxDispatcher(
        backend = ExposedOutboxBackend(),
        publisher = publisher,
        pollInterval = Duration.ofSeconds(config.outboxPollSeconds),
        batchSize = config.outboxBatchSize,
        metrics = metrics,
    )

    val grpcService = PaymentsGrpcService(payments, dispatcher)
    val health = HealthCheck()

    val server = ServerBuilder.forPort(config.grpcPort)
        .addService(grpcService)
        .addService(health.service)
        .build()

    val httpServer = startHttpServer(config.httpPort, registry)
    server.start()
    health.setServing()
    health.setServing("atlas.payments.PaymentsService")
    dispatcher.start()
    LOG.info("started gRPC server on :{} and HTTP server on :{}", config.grpcPort, config.httpPort)

    Runtime.getRuntime().addShutdownHook(Thread {
        LOG.info("shutdown signal received, draining")
        health.setNotServing()
        health.shutdown()
        dispatcher.close()
        try {
            server.shutdown().awaitTermination(15, TimeUnit.SECONDS)
        } catch (e: InterruptedException) {
            Thread.currentThread().interrupt()
        }
        httpServer.stop(1_000, 5_000)
        publisher.close()
        LOG.info("payments-service stopped")
    })

    server.awaitTermination()
}
