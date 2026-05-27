package io.atlas.auth.core

/**
 * Sealed hierarchy of auth-domain errors. The gRPC layer (Phase 2B) maps
 * each subtype to the appropriate status code; this layer is transport-
 * agnostic.
 *
 * [InvalidCredentials] deliberately does not distinguish "unknown email"
 * from "wrong password" so the API cannot be used to enumerate registered
 * emails.
 */
sealed class AuthError(message: String) : Exception(message) {
    class EmailAlreadyExists(email: String) : AuthError("email already registered: $email")
    class InvalidEmail(email: String) : AuthError("invalid email address: $email")
    class WeakPassword(reason: String) : AuthError("weak password: $reason")
    class InvalidCredentials : AuthError("invalid email or password")
    class TokenInvalid(reason: String) : AuthError("token invalid: $reason")
    class SessionRevoked : AuthError("session has been revoked")
    class SessionExpired : AuthError("session has expired")
}
