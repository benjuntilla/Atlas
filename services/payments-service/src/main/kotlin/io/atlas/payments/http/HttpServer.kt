package io.atlas.payments.http

import io.atlas.payments.core.PaymentProvider
import io.atlas.payments.core.OutboxBackend
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
fun startHttpServer(
    port: Int,
    registry: PrometheusMeterRegistry,
    provider: PaymentProvider,
): NettyApplicationEngine =
    embeddedServer(Netty, port = port) {
        routing {
            get("/metrics") {
                call.respondText(registry.scrape(), ContentType.Text.Plain)
            }
            get("/healthz") {
                call.respondText("ok")
            }
            post("/webhooks/{provider}") {
                val source = call.parameters["provider"] ?: "unknown"
                val body = call.receiveText()

                // Verify BEFORE doing anything with the payload. This
                // endpoint is an unauthenticated write path from the public
                // internet into a payments system; the only thing making it
                // safe is that the provider signed the request.
                //
                // The check runs even though FakePaymentProvider accepts
                // everything, so the call site exists and cannot be
                // forgotten when a real provider is wired in. Verification
                // is provider-specific, which is why it lives on the
                // PaymentProvider interface rather than here.
                val signature = call.request.headers["Stripe-Signature"]
                    ?: call.request.headers["X-Webhook-Signature"]
                if (!provider.verifyWebhook(body, signature)) {
                    LOG.warn("rejected webhook from provider={}: bad signature", source)
                    call.respond(HttpStatusCode.Unauthorized)
                    return@post
                }

                // Acknowledged and logged. Reconciling the referenced charge
                // against payments.transactions is the next step and needs a
                // real provider's event schema to be worth writing.
                LOG.info("accepted webhook from provider={} bytes={}", source, body.length)
                call.respond(HttpStatusCode.OK)
            }
        }
    }.start(wait = false)

/** Creates the Prometheus registry App.kt shares with the HTTP server and metrics. */
fun newPrometheusRegistry(): PrometheusMeterRegistry =
    PrometheusMeterRegistry(PrometheusConfig.DEFAULT)

/** Micrometer-backed [PaymentsMetrics] exposed via /metrics. */
/**
 * Register outbox depth as GAUGES fed by a supplier.
 *
 * Gauges rather than counters because the question is "how much is stuck
 * right now", and Micrometer polls the supplier at scrape time so the
 * value is never stale.
 *
 * This is the signal payments actually needs. `outbox_dispatched_total`
 * is a counter of SUCCESSES: when Kafka is unreachable it simply stops
 * increasing, and a counter that stops looks exactly like a system with
 * nothing to do. Depth goes UP when the drain is stuck, which is a
 * statement rather than an absence.
 *
 * Two series, because they answer different questions: row count says how
 * much is waiting, and the age of the oldest row distinguishes "busy"
 * from "wedged" — a large backlog that is draining has a young head.
 */
fun registerOutboxGauges(registry: MeterRegistry, backend: OutboxBackend) {
    registry.gauge("atlas_payments_outbox_pending_rows", backend) {
        it.pending().rows.toDouble()
    }
    registry.gauge("atlas_payments_outbox_oldest_age_seconds", backend) {
        it.pending().oldestAgeSeconds.toDouble()
    }
}

class MicrometerPaymentsMetrics(registry: MeterRegistry) : PaymentsMetrics {
    private val initiated = registry.counter("atlas_payments_transactions_initiated_total")
    private val settled = registry.counter("atlas_payments_transactions_settled_total")
    private val refunded = registry.counter("atlas_payments_transactions_refunded_total")
    private val dispatched = registry.counter("atlas_payments_outbox_dispatched_total")
    private val depositOk = registry.counter("atlas_payments_deposits_total", "outcome", "settled")
    private val depositFail = registry.counter("atlas_payments_deposits_total", "outcome", "failed")

    override fun transactionInitiated() = initiated.increment()
    override fun transactionSettled() = settled.increment()
    override fun transactionRefunded() = refunded.increment()
    override fun outboxDispatched(count: Int) = dispatched.increment(count.toDouble())
    override fun depositSettled() = depositOk.increment()
    override fun depositFailed() = depositFail.increment()
}
