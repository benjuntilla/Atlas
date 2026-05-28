package io.atlas.auth.grpc

import io.grpc.health.v1.HealthCheckResponse
import io.grpc.protobuf.services.HealthStatusManager

/**
 * Thin wrapper around grpc-services' [HealthStatusManager] so [App.kt] can
 * register the standard `grpc.health.v1.Health` service on the server and
 * flip status to NOT_SERVING during graceful shutdown.
 *
 * Kubernetes liveness/readiness probes use `grpc_health_probe` against this
 * service in later phases; doing the wiring now means no migration later.
 */
class HealthCheck {
    private val manager = HealthStatusManager()

    val service get() = manager.healthService

    fun setServing(serviceName: String = "") {
        manager.setStatus(serviceName, HealthCheckResponse.ServingStatus.SERVING)
    }

    fun setNotServing(serviceName: String = "") {
        manager.setStatus(serviceName, HealthCheckResponse.ServingStatus.NOT_SERVING)
    }

    fun shutdown() {
        manager.enterTerminalState()
    }
}
