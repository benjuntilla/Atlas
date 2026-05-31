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

    companion object {
        val NOOP: PaymentsMetrics = object : PaymentsMetrics {
            override fun transactionInitiated() {}
            override fun transactionSettled() {}
            override fun transactionRefunded() {}
            override fun outboxDispatched(count: Int) {}
        }
    }
}
