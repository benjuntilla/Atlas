package io.atlas.auth.grpc

import atlas.auth.AuthRequest
import atlas.auth.AuthServiceGrpcKt
import atlas.auth.IssueTokenRequest
import atlas.auth.RegisterRequest
import atlas.auth.RegisterResponse
import atlas.auth.RevokeResponse
import atlas.auth.RevokeTokenRequest
import atlas.auth.TokenResponse
import atlas.auth.ValidateTokenRequest
import io.atlas.auth.cache.TokenValidationCache
import io.atlas.auth.core.AuthError
import io.atlas.auth.core.AuthService
import io.atlas.auth.core.SessionRepository
import io.atlas.auth.core.SignedToken
import io.atlas.auth.crypto.JwtSigner
import io.atlas.auth.kafka.AuthTokenEventPublisher
import io.atlas.auth.kafka.sha256Hex
import io.grpc.Status
import io.grpc.StatusException
import org.slf4j.LoggerFactory
import java.time.Clock
import java.util.UUID
import io.atlas.auth.core.TokenClaims as DomainTokenClaims
import atlas.auth.TokenClaims as ProtoTokenClaims

/**
 * gRPC fronting [AuthService]. Translates the domain API into the proto
 * contract defined in proto/auth.proto, maps domain errors to gRPC status
 * codes, and threads two side effects through every call:
 *
 *  1. A 30s in-process [TokenValidationCache] so ValidateToken does not hit
 *     Postgres on every gateway request. The cache is keyed by JWT signature
 *     segment.
 *  2. Kafka [AuthTokenEventPublisher] fanout on Authenticate, IssueToken,
 *     and RevokeToken so other instances can evict their caches and so
 *     downstream consumers see token lifecycle. Register does not produce a
 *     token, so it does not publish an AuthTokenEvent.
 */
class AuthGrpcService(
    private val authService: AuthService,
    private val sessions: SessionRepository,
    private val signer: JwtSigner,
    private val cache: TokenValidationCache,
    private val publisher: AuthTokenEventPublisher,
    private val clock: Clock = Clock.systemUTC(),
) : AuthServiceGrpcKt.AuthServiceCoroutineImplBase() {

    override suspend fun register(request: RegisterRequest): RegisterResponse {
        val userId = try {
            authService.register(request.email, request.password)
        } catch (e: AuthError) {
            throw e.toGrpcStatusException()
        }
        return RegisterResponse.newBuilder().setUserId(userId.toString()).build()
    }

    override suspend fun authenticate(request: AuthRequest): TokenResponse {
        val signed = try {
            authService.authenticate(
                email = request.email,
                password = request.password,
                lastLat = if (request.lat == 0.0 && request.lng == 0.0) null else request.lat,
                lastLng = if (request.lat == 0.0 && request.lng == 0.0) null else request.lng,
            )
        } catch (e: AuthError) {
            throw e.toGrpcStatusException()
        }
        publishIssued(signed)
        return signed.toResponse()
    }

    override suspend fun issueToken(request: IssueTokenRequest): TokenResponse {
        val userId = try {
            UUID.fromString(request.userId)
        } catch (e: IllegalArgumentException) {
            throw Status.INVALID_ARGUMENT.withDescription("user_id is not a UUID").asException()
        }
        val signed = try {
            authService.issueTokenForUser(
                userId = userId,
                lastLat = if (request.lat == 0.0 && request.lng == 0.0) null else request.lat,
                lastLng = if (request.lat == 0.0 && request.lng == 0.0) null else request.lng,
            )
        } catch (e: AuthError) {
            throw e.toGrpcStatusException()
        }
        publishIssued(signed)
        return signed.toResponse()
    }

    override suspend fun validateToken(request: ValidateTokenRequest): ProtoTokenClaims {
        val token = request.token
        cache.get(token)?.let { return it.toProto() }

        val domainClaims = try {
            signer.verify(token)
        } catch (e: AuthError) {
            throw e.toGrpcStatusException()
        }

        val session = sessions.findById(domainClaims.sessionId)
            ?: throw Status.UNAUTHENTICATED.withDescription("session not found").asException()
        if (session.revoked) {
            throw Status.PERMISSION_DENIED.withDescription("session revoked").asException()
        }
        if (clock.instant().isAfter(session.expiresAt)) {
            throw Status.UNAUTHENTICATED.withDescription("session expired").asException()
        }

        cache.putWithHash(token, domainClaims, sha256Hex(token))
        return domainClaims.toProto()
    }

    override suspend fun revokeToken(request: RevokeTokenRequest): RevokeResponse {
        val token = request.token
        val domainClaims = try {
            signer.verify(token)
        } catch (e: AuthError) {
            // Revoking an already-invalid token is idempotent and not an
            // error from the caller's perspective; surface success=false.
            return RevokeResponse.newBuilder().setSuccess(false).build()
        }
        sessions.revoke(domainClaims.sessionId)
        cache.evict(token)
        try {
            publisher.publishRevoked(domainClaims, token)
        } catch (e: Exception) {
            LOG.warn("publishRevoked threw; revocation still persisted", e)
        }
        return RevokeResponse.newBuilder().setSuccess(true).build()
    }

    // --- helpers ----------------------------------------------------------

    private fun publishIssued(signed: SignedToken) {
        val claims = try {
            signer.verify(signed.token)
        } catch (e: AuthError) {
            // We just signed it; this is unreachable barring a config bug.
            LOG.error("could not verify token we just issued", e)
            return
        }
        cache.putWithHash(signed.token, claims, sha256Hex(signed.token))
        try {
            publisher.publishIssued(claims, signed.token)
        } catch (e: Exception) {
            LOG.warn("publishIssued threw; token still valid", e)
        }
    }

    companion object {
        private val LOG = LoggerFactory.getLogger(AuthGrpcService::class.java)
    }
}

private fun SignedToken.toResponse(): TokenResponse =
    TokenResponse.newBuilder()
        .setToken(token)
        .setExpiresAt(expiresAt.epochSecond)
        .build()

private fun DomainTokenClaims.toProto(): ProtoTokenClaims =
    ProtoTokenClaims.newBuilder()
        .setUserId(userId.toString())
        .setSessionId(sessionId.toString())
        .setIssuedAt(issuedAt.epochSecond)
        .setExpiresAt(expiresAt.epochSecond)
        .setLastLat(lastLat ?: 0.0)
        .setLastLng(lastLng ?: 0.0)
        .build()

private fun AuthError.toGrpcStatusException(): StatusException = when (this) {
    is AuthError.EmailAlreadyExists -> Status.ALREADY_EXISTS.withDescription(message).asException()
    is AuthError.InvalidEmail -> Status.INVALID_ARGUMENT.withDescription(message).asException()
    is AuthError.WeakPassword -> Status.INVALID_ARGUMENT.withDescription(message).asException()
    is AuthError.InvalidCredentials -> Status.UNAUTHENTICATED.withDescription(message).asException()
    is AuthError.TokenInvalid -> Status.UNAUTHENTICATED.withDescription(message).asException()
    is AuthError.SessionRevoked -> Status.PERMISSION_DENIED.withDescription(message).asException()
    is AuthError.SessionExpired -> Status.UNAUTHENTICATED.withDescription(message).asException()
}
