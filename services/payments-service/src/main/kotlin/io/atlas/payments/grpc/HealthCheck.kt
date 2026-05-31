package io.atlas.payments.grpc

import io.grpc.health.v1.HealthCheckResponse
import io.grpc.protobuf.services.HealthStatusManager

/**
 * Thin wrapper around grpc-services' [HealthStatusManager] so App.kt can
 * register `grpc.health.v1.Health` and flip status to NOT_SERVING during
 * graceful shutdown. Kubernetes probes use `grpc_health_probe` against this in
 * later phases.
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
