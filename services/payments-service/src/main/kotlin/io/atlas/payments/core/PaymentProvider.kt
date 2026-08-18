package io.atlas.payments.core

import java.util.UUID

/**
 * Abstraction over an external payment processor (Stripe, Adyen, ...).
 *
 * # Everything real except the network call
 *
 * The platform deliberately runs against [FakePaymentProvider]. That is a
 * placeholder for the *provider call only* — not for the machinery around
 * it. Idempotency, the pending-then-capture recovery window, the
 * transactional outbox, the ledger updates and the webhook path are all
 * real and exercised, because those are the parts that are expensive to
 * get wrong and expensive to retrofit.
 *
 * Swapping in a real processor means implementing this interface and
 * changing one environment variable. Nothing above it moves. Stripe's test
 * mode is free, so even that work costs nothing until live keys are used.
 *
 * # Contract
 *
 * Provider calls are made OUTSIDE the database transaction: network I/O
 * must never hold a Postgres row lock. Implementations must be safe to
 * retry with the same idempotency key — [authorize] is called with the
 * caller's key precisely so a retry does not double-charge.
 */
data class ProviderResult(
    val success: Boolean,
    val providerRef: String,
    val message: String? = null,
)

interface PaymentProvider {
    /** Human-readable name, surfaced in logs and on the health endpoint. */
    val name: String

    /** Reserve [amountCents] against the payer. Returns the provider reference. */
    fun authorize(amountCents: Long, idempotencyKey: String): ProviderResult

    /** Capture a previously authorized charge. */
    fun capture(providerRef: String): ProviderResult

    /** Reverse a captured charge. */
    fun refund(providerRef: String): ProviderResult

    /**
     * Verify that a webhook body genuinely came from the provider.
     *
     * This exists on the interface rather than in the HTTP layer because
     * the verification scheme is provider-specific — Stripe signs with an
     * HMAC over a timestamped payload, others use mTLS or a shared token.
     * Putting it here means the endpoint cannot accidentally ship without
     * a check: it has something to call from day one, and the real
     * implementation slots in behind it.
     *
     * An unverified webhook is an unauthenticated write from the public
     * internet into a payments system, so the endpoint MUST reject when
     * this returns false.
     */
    fun verifyWebhook(payload: String, signature: String?): Boolean
}

/**
 * Always-approve provider for local development and tests.
 *
 * Generates a unique `fake_*` reference on authorize and echoes it back on
 * capture and refund, so the full authorize -> capture -> refund flow is
 * exercised end to end without a network call or a cent of real money.
 */
class FakePaymentProvider : PaymentProvider {
    override val name: String = "fake"

    override fun authorize(amountCents: Long, idempotencyKey: String): ProviderResult =
        ProviderResult(success = true, providerRef = "fake_${UUID.randomUUID()}")

    override fun capture(providerRef: String): ProviderResult =
        ProviderResult(success = true, providerRef = providerRef)

    override fun refund(providerRef: String): ProviderResult =
        ProviderResult(success = true, providerRef = providerRef)

    /**
     * Accepts anything.
     *
     * Safe only because this provider never sends webhooks, so nothing
     * legitimate calls the endpoint at all. It is NOT a stand-in for real
     * verification: shipping this against a live processor would leave an
     * unauthenticated write path into payments.
     */
    override fun verifyWebhook(payload: String, signature: String?): Boolean = true
}

/**
 * Chooses the provider from configuration.
 *
 * Unknown values fail loudly at startup rather than silently falling back
 * to the fake. A service that quietly runs on a stub provider in
 * production because of a typo in an environment variable would approve
 * every charge and move real balances against money that was never
 * collected.
 */
object PaymentProviders {
    fun fromName(name: String): PaymentProvider = when (name.trim().lowercase()) {
        "", "fake" -> FakePaymentProvider()
        "stripe" -> throw IllegalArgumentException(
            "PAYMENT_PROVIDER=stripe is not implemented yet. Implement PaymentProvider " +
                "against the Stripe SDK and register it here; nothing above this " +
                "interface needs to change.",
        )
        else -> throw IllegalArgumentException(
            "unknown PAYMENT_PROVIDER '$name'. Known: fake",
        )
    }
}
