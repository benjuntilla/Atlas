package io.atlas.auth.core

import java.time.Instant
import java.util.UUID

/**
 * Domain types for the auth service. These are deliberately decoupled from
 * both the DB row schema and the wire (gRPC) types. The persistence layer
 * maps to and from [User] and [Session]; the gRPC layer maps to and from
 * [TokenClaims] and the request/response messages.
 */

data class User(
    val id: UUID,
    /** The project this user belongs to. Users are scoped, never global. */
    val projectId: UUID,
    val email: String,
    val passwordHash: String,
    val createdAt: Instant,
)

data class Session(
    val id: UUID,
    val userId: UUID,
    val issuedAt: Instant,
    val expiresAt: Instant,
    val revoked: Boolean,
)

/**
 * The validated payload of an Atlas JWT. Maps 1:1 to the claims in
 * [io.atlas.auth.crypto.JwtSigner]. `lastLat`/`lastLng` are optional because
 * a token can be issued without a known location (e.g. during signup before
 * geolocation permission is granted).
 */
data class TokenClaims(
    val userId: UUID,
    /**
     * Signed into the token at issue time so it cannot be swapped later.
     * The gateway compares this against the project its API key resolved
     * to, which is what stops a token minted in one project from being
     * replayed against another.
     */
    val projectId: UUID,
    val sessionId: UUID,
    val issuedAt: Instant,
    val expiresAt: Instant,
    val lastLat: Double?,
    val lastLng: Double?,
)

/**
 * Returned by [AuthService.authenticate]. Holds the encoded token and its
 * expiry so the caller does not need to re-decode the JWT to know when to
 * refresh.
 */
data class SignedToken(
    val token: String,
    val expiresAt: Instant,
)
