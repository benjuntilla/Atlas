package io.atlas.payments.core

import java.util.UUID

/**
 * Abstraction over an external payment processor (Stripe, Adyen, etc). Atlas
 * never calls a real provider in Phase 4 - that would mean signing up for an
 * account and spending money, which violates the $0-through-Phase-15 rule. The
 * [FakePaymentProvider] approves everything and mints deterministic references
 * so the full authorize -> capture -> refund flow is exercised end to end.
 *
 * Provider calls are made OUTSIDE the database transaction (network I/O must
 * not hold a Postgres row lock), then the resulting reference is persisted.
 */
data class ProviderResult(
    val success: Boolean,
    val providerRef: String,
    val message: String? = null,
)

interface PaymentProvider {
    /** Reserve [amountCents] against the payer. Returns the provider reference. */
    fun authorize(amountCents: Long, idempotencyKey: String): ProviderResult

    /** Capture a previously authorized charge. */
    fun capture(providerRef: String): ProviderResult

    /** Reverse a captured charge. */
    fun refund(providerRef: String): ProviderResult
}

/**
 * Always-approve provider for local dev and tests. Generates a unique
 * `fake_*` reference on authorize and echoes it back on capture/refund.
 */
class FakePaymentProvider : PaymentProvider {
    override fun authorize(amountCents: Long, idempotencyKey: String): ProviderResult =
        ProviderResult(success = true, providerRef = "fake_${UUID.randomUUID()}")

    override fun capture(providerRef: String): ProviderResult =
        ProviderResult(success = true, providerRef = providerRef)

    override fun refund(providerRef: String): ProviderResult =
        ProviderResult(success = true, providerRef = providerRef)
}
