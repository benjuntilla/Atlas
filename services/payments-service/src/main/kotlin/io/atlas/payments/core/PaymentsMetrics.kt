package io.atlas.payments.core

/**
 * Counters exposed on the /metrics endpoint. The service and dispatcher depend
 * on this interface so unit tests can pass [NOOP] and production passes a
 * Micrometer-backed implementation (see io.atlas.payments.http).
 */
interface PaymentsMetrics {
    fun transactionInitiated()
    fun transactionSettled()
    fun transactionRefunded()
    fun outboxDispatched(count: Int)

    /**
     * One reconciliation pass.
     *
     * `unresolved` is the number worth alerting on: settled and failed are
     * the sweep doing its job, while unresolved is money whose fate nobody
     * knows.
     */
    fun reconciled(settled: Int, failed: Int, unresolved: Int)

    /** A deposit that credited a wallet. This is money entering the platform. */
    fun depositSettled()

    /**
     * A deposit the provider refused at capture. Worth alerting on: a rise
     * here is either a provider incident or a card-testing attack.
     */
    fun depositFailed()

    companion object {
        val NOOP: PaymentsMetrics = object : PaymentsMetrics {
            override fun transactionInitiated() {}
            override fun transactionSettled() {}
            override fun transactionRefunded() {}
            override fun outboxDispatched(count: Int) {}
            override fun reconciled(settled: Int, failed: Int, unresolved: Int) {}
            override fun depositSettled() {}
            override fun depositFailed() {}
        }
    }
}
