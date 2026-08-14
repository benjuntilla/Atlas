package io.atlas.fare.core

/**
 * Counters for what the handler decided, mirroring the PaymentsMetrics
 * shape in payments-service (interface + Micrometer impl + NOOP) so tests
 * can construct a handler without a registry.
 *
 * [rejected] and [unresolved] are the two worth alerting on. Both are
 * "normal" in small numbers — a cancelled ride that was never captured, a
 * ride completing without a fare — but a sustained rate means the
 * application is producing events Atlas cannot act on.
 */
interface FareMetrics {
    fun settled()
    fun refunded()
    fun rejected()
    fun retried()
    fun unresolved()
    fun duplicate()

    companion object {
        val NOOP: FareMetrics = object : FareMetrics {
            override fun settled() = Unit
            override fun refunded() = Unit
            override fun rejected() = Unit
            override fun retried() = Unit
            override fun unresolved() = Unit
            override fun duplicate() = Unit
        }
    }
}
