package io.atlas.payments.http

import io.atlas.payments.core.PaymentsMetrics
import io.ktor.http.ContentType
import io.ktor.http.HttpStatusCode
import io.ktor.server.application.call
import io.ktor.server.engine.embeddedServer
import io.ktor.server.netty.Netty
import io.ktor.server.netty.NettyApplicationEngine
import io.ktor.server.request.receiveText
import io.ktor.server.response.respond
import io.ktor.server.response.respondText
import io.ktor.server.routing.get
import io.ktor.server.routing.post
import io.ktor.server.routing.routing
import io.micrometer.core.instrument.MeterRegistry
import io.micrometer.prometheusmetrics.PrometheusConfig
import io.micrometer.prometheusmetrics.PrometheusMeterRegistry
import org.slf4j.LoggerFactory

private val LOG = LoggerFactory.getLogger("io.atlas.payments.http.HttpServer")

/**
 * Single Ktor/Netty HTTP server serving both the Prometheus scrape endpoint and
 * provider webhooks. Consolidating them onto one port keeps the service to one
 * HTTP listener (gRPC is separate, on its own port).
 *
 *   GET  /metrics            Prometheus exposition (Micrometer)
 *   GET  /healthz            liveness ping
 *   POST /webhooks/{provider} async provider callbacks (acknowledged, logged)
 */
fun startHttpServer(port: Int, registry: PrometheusMeterRegistry): NettyApplicationEngine =
    embeddedServer(Netty, port = port) {
        routing {
            get("/metrics") {
                call.respondText(registry.scrape(), ContentType.Text.Plain)
            }
            get("/healthz") {
                call.respondText("ok")
            }
            post("/webhooks/{provider}") {
                val provider = call.parameters["provider"] ?: "unknown"
                val body = call.receiveText()
                // Phase 4 uses the FakePaymentProvider, which never calls back.
                // Real providers (Stripe et al.) would verify a signature and
                // reconcile the referenced charge here. We acknowledge so the
                // provider does not retry, and log for observability.
                LOG.info("received webhook from provider={} bytes={}", provider, body.length)
                call.respond(HttpStatusCode.OK)
            }
        }
    }.start(wait = false)

/** Creates the Prometheus registry App.kt shares with the HTTP server and metrics. */
fun newPrometheusRegistry(): PrometheusMeterRegistry =
    PrometheusMeterRegistry(PrometheusConfig.DEFAULT)

/** Micrometer-backed [PaymentsMetrics] exposed via /metrics. */
class MicrometerPaymentsMetrics(registry: MeterRegistry) : PaymentsMetrics {
    private val initiated = registry.counter("atlas_payments_transactions_initiated_total")
    private val settled = registry.counter("atlas_payments_transactions_settled_total")
    private val refunded = registry.counter("atlas_payments_transactions_refunded_total")
    private val dispatched = registry.counter("atlas_payments_outbox_dispatched_total")

    override fun transactionInitiated() = initiated.increment()
    override fun transactionSettled() = settled.increment()
    override fun transactionRefunded() = refunded.increment()
    override fun outboxDispatched(count: Int) = dispatched.increment(count.toDouble())
}
