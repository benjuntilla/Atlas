package io.atlas.fare.http

import io.atlas.fare.core.FareMetrics
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
 * Ktor/Netty server for the scrape endpoint and a liveness ping. This
 * process has no API of its own — it is a Kafka worker — so these two
 * routes are the whole HTTP surface.
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

fun newPrometheusRegistry(): PrometheusMeterRegistry =
    PrometheusMeterRegistry(PrometheusConfig.DEFAULT)

class MicrometerFareMetrics(registry: MeterRegistry) : FareMetrics {
    private val settled = registry.counter("atlas_fare_settlements_total")
    private val refunded = registry.counter("atlas_fare_refunds_total")
    private val rejected = registry.counter("atlas_fare_rejected_total")
    private val retried = registry.counter("atlas_fare_retried_total")
    private val unresolved = registry.counter("atlas_fare_unresolved_total")
    private val duplicate = registry.counter("atlas_fare_duplicate_events_total")

    override fun settled() = settled.increment()
    override fun refunded() = refunded.increment()
    override fun rejected() = rejected.increment()
    override fun retried() = retried.increment()
    override fun unresolved() = unresolved.increment()
    override fun duplicate() = duplicate.increment()
}
