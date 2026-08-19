package io.atlas.auth.http

import io.atlas.auth.core.AuthMetrics
import io.ktor.http.ContentType
import io.ktor.server.application.call
import io.ktor.server.engine.embeddedServer
import io.ktor.server.netty.Netty
import io.ktor.server.netty.NettyApplicationEngine
import io.ktor.server.response.respondText
import io.ktor.server.routing.get
import io.ktor.server.routing.routing
import io.micrometer.core.instrument.MeterRegistry
import io.micrometer.prometheusmetrics.PrometheusConfig
import io.micrometer.prometheusmetrics.PrometheusMeterRegistry

/**
 * Ktor/Netty HTTP server for the Prometheus scrape endpoint, on its own port
 * separate from gRPC.
 *
 *   GET /metrics   Prometheus exposition (Micrometer)
 *   GET /healthz   liveness ping
 *
 * gRPC clients keep using grpc.health.v1.Health; /healthz exists for HTTP
 * probes and for parity with payments-service.
 *
 * This port must never be exposed publicly — request volumes and auth
 * failure rates are exactly what an attacker would like to watch.
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
        }
    }.start(wait = false)

/** Registry shared by App.kt and the HTTP server. */
fun newPrometheusRegistry(): PrometheusMeterRegistry =
    PrometheusMeterRegistry(PrometheusConfig.DEFAULT)

/**
 * Micrometer-backed [AuthMetrics].
 *
 * Counters with a label are resolved per call rather than cached as fields,
 * because the label value varies. The reason strings come from a closed set
 * of domain error kinds, so cardinality stays bounded — this is why the
 * interface takes a reason kind and not an arbitrary message.
 */
class MicrometerAuthMetrics(private val registry: MeterRegistry) : AuthMetrics {
    private val registered = registry.counter("atlas_auth_users_registered_total")

    // Every series of `atlas_auth_authentications_total` must carry the same
    // label KEYS. Registering success as {outcome} and failure as
    // {outcome,reason} is invalid in Prometheus, and Micrometer resolves the
    // conflict by silently dropping the second registration — the failure
    // counter simply never appears on /metrics, which is the worst possible
    // way to lose an alerting signal. So success carries reason="none".
    private val authenticatedOk =
        registry.counter("atlas_auth_authentications_total", "outcome", "success", "reason", "none")
    private val issued = registry.counter("atlas_auth_tokens_issued_total")
    private val validatedHit = registry.counter("atlas_auth_tokens_validated_total", "cache", "hit")
    private val validatedMiss = registry.counter("atlas_auth_tokens_validated_total", "cache", "miss")
    private val revoked = registry.counter("atlas_auth_tokens_revoked_total")

    // The gap between requested and completed is the interesting signal:
    // resets asked for but never finished mean links are not arriving.
    private val resetRequested = registry.counter("atlas_auth_password_resets_requested_total")
    private val resetCompleted = registry.counter("atlas_auth_password_resets_completed_total")
    private val emailsVerified = registry.counter("atlas_auth_emails_verified_total")

    override fun userRegistered() = registered.increment()

    override fun authenticated() = authenticatedOk.increment()

    override fun authenticationFailed(reason: String) =
        registry.counter("atlas_auth_authentications_total", "outcome", "failure", "reason", reason)
            .increment()

    override fun tokenIssued() = issued.increment()

    override fun tokenValidated(cacheHit: Boolean) =
        if (cacheHit) validatedHit.increment() else validatedMiss.increment()

    override fun tokenRejected(reason: String) =
        registry.counter("atlas_auth_tokens_rejected_total", "reason", reason).increment()

    override fun tokenRevoked() = revoked.increment()

    override fun passwordResetRequested() = resetRequested.increment()

    override fun passwordReset() = resetCompleted.increment()

    // Same label-key rule as authentications above: this counter always
    // carries `reason`, so the rejection series and any future success
    // series stay compatible.
    override fun passwordResetRejected(reason: String) =
        registry.counter("atlas_auth_password_resets_rejected_total", "reason", reason)
            .increment()

    override fun emailVerified() = emailsVerified.increment()

    override fun tokenEventPublishFailed(reason: String) =
        registry.counter("atlas_auth_token_events_failed_total", "reason", reason).increment()
}
