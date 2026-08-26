package io.atlas.auth.core

/**
 * Counters exposed on the /metrics endpoint. Mirrors the shape of
 * `PaymentsMetrics`: the gRPC layer depends on this interface so tests can
 * pass [NOOP] and production passes the Micrometer-backed implementation in
 * `io.atlas.auth.http`.
 *
 * # Why these particular counters
 *
 * auth-service sits on the hottest path in the platform — the gateway calls
 * ValidateToken on every authenticated request — and until now it was the
 * only service exposing no metrics at all. In production that meant no way
 * to answer "is auth slow?" or "are we hammering Postgres?" without a
 * debugger.
 *
 * The cache hit rate is the most operationally important number here. Every
 * miss is a Postgres round trip on the request path, so a hit rate that
 * drops is the earliest warning that the database is about to become the
 * bottleneck. [tokenValidated] takes the outcome so the ratio is derivable
 * rather than needing two uncorrelated counters.
 *
 * Authentication failures are counted separately from successes because a
 * spike in failures with flat successes is what credential stuffing looks
 * like.
 */
interface AuthMetrics {
    fun userRegistered()

    /** A successful password authentication. */
    fun authenticated()

    /**
     * A rejected authentication. [reason] is a low-cardinality label — the
     * domain error kind, never an email address or a token.
     */
    fun authenticationFailed(reason: String)

    fun tokenIssued()

    /** [cacheHit] false means this validation cost a Postgres read. */
    fun tokenValidated(cacheHit: Boolean)

    fun tokenRejected(reason: String)

    fun tokenRevoked()

    /** Requested, whether or not the address existed — see the RPC doc. */
    fun passwordResetRequested()
    fun passwordReset()
    fun passwordResetRejected(reason: String)
    fun emailVerified()

    /**
     * A token lifecycle event that never reached Kafka. [reason] is one of a
     * small fixed set: delivery, send, rejected, queue_full.
     *
     * Worth alerting on. Missed events mean other instances do not evict
     * revoked tokens from their caches, so a revoked token can stay usable
     * on another replica until its 30s TTL expires.
     */
    fun tokenEventPublishFailed(reason: String)

    companion object {
        val NOOP: AuthMetrics = object : AuthMetrics {
            override fun userRegistered() {}
            override fun authenticated() {}
            override fun authenticationFailed(reason: String) {}
            override fun tokenIssued() {}
            override fun tokenValidated(cacheHit: Boolean) {}
            override fun tokenRejected(reason: String) {}
            override fun tokenRevoked() {}
            override fun passwordResetRequested() {}
            override fun passwordReset() {}
            override fun passwordResetRejected(reason: String) {}
            override fun emailVerified() {}
            override fun tokenEventPublishFailed(reason: String) {}
        }
    }
}
