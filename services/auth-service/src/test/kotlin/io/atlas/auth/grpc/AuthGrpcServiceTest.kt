package io.atlas.auth.grpc

import atlas.auth.AuthRequest
import atlas.auth.IssueTokenRequest
import atlas.auth.RegisterRequest
import atlas.auth.RevokeTokenRequest
import atlas.auth.ValidateTokenRequest
import atlas.events.AuthTokenEvent
import io.atlas.auth.cache.TokenValidationCache
import io.atlas.auth.core.AuthService
import io.atlas.auth.crypto.BcryptPasswordHasher
import io.atlas.auth.crypto.Jose4jJwtSigner
import io.atlas.auth.kafka.RecordingAuthTokenPublisher
import io.atlas.auth.memory.InMemorySessionRepository
import io.atlas.auth.memory.InMemoryUserRepository
import io.grpc.Status
import io.grpc.StatusException
import kotlinx.coroutines.test.runTest
import org.junit.jupiter.api.Test
import java.time.Clock
import java.time.Duration
import java.time.Instant
import java.time.ZoneOffset
import java.util.UUID
import kotlin.test.assertEquals
import kotlin.test.assertFailsWith
import kotlin.test.assertFalse
import kotlin.test.assertNotNull
import kotlin.test.assertTrue

class AuthGrpcServiceTest {

    private fun newHarness(
        now: Instant = Instant.now().truncatedTo(java.time.temporal.ChronoUnit.SECONDS),
    ): Harness {
        val clock = Clock.fixed(now, ZoneOffset.UTC)
        val users = InMemoryUserRepository(clock = clock)
        val sessions = InMemorySessionRepository()
        val hasher = BcryptPasswordHasher(cost = 4)
        val signer = Jose4jJwtSigner("a-very-secret-secret-of-sufficient-length")
        val authService = AuthService(
            users = users,
            sessions = sessions,
            hasher = hasher,
            signer = signer,
            tokenLifetime = Duration.ofHours(1),
            clock = clock,
        )
        val cache = TokenValidationCache()
        val publisher = RecordingAuthTokenPublisher()
        val grpc = AuthGrpcService(
            authService = authService,
            sessions = sessions,
            signer = signer,
            cache = cache,
            publisher = publisher,
            clock = clock,
        )
        return Harness(grpc, sessions, cache, publisher, now)
    }

    private data class Harness(
        val grpc: AuthGrpcService,
        val sessions: InMemorySessionRepository,
        val cache: TokenValidationCache,
        val publisher: RecordingAuthTokenPublisher,
        val now: Instant,
    )

    // --- register ---------------------------------------------------------

    @Test
    fun `register returns a UUID on success`() = runTest {
        val h = newHarness()
        val resp = h.grpc.register(
            RegisterRequest.newBuilder()
                .setEmail("alice@example.com")
                .setPassword("correct-horse-battery-staple")
                .build()
        )
        // Parses as UUID.
        UUID.fromString(resp.userId)
    }

    @Test
    fun `register duplicate maps to ALREADY_EXISTS`() = runTest {
        val h = newHarness()
        val req = RegisterRequest.newBuilder()
            .setEmail("alice@example.com")
            .setPassword("correct-horse-battery-staple")
            .build()
        h.grpc.register(req)
        val ex = assertFailsWith<StatusException> { h.grpc.register(req) }
        assertEquals(Status.ALREADY_EXISTS.code, ex.status.code)
    }

    @Test
    fun `register invalid email maps to INVALID_ARGUMENT`() = runTest {
        val h = newHarness()
        val ex = assertFailsWith<StatusException> {
            h.grpc.register(
                RegisterRequest.newBuilder().setEmail("not-an-email").setPassword("correct-horse-battery-staple").build()
            )
        }
        assertEquals(Status.INVALID_ARGUMENT.code, ex.status.code)
    }

    @Test
    fun `register weak password maps to INVALID_ARGUMENT`() = runTest {
        val h = newHarness()
        val ex = assertFailsWith<StatusException> {
            h.grpc.register(
                RegisterRequest.newBuilder().setEmail("u@x.com").setPassword("short").build()
            )
        }
        assertEquals(Status.INVALID_ARGUMENT.code, ex.status.code)
    }

    // --- authenticate -----------------------------------------------------

    @Test
    fun `authenticate happy path returns token, publishes ISSUED, caches`() = runTest {
        val h = newHarness()
        h.grpc.register(
            RegisterRequest.newBuilder()
                .setEmail("alice@example.com")
                .setPassword("correct-horse-battery-staple")
                .build()
        )
        val resp = h.grpc.authenticate(
            AuthRequest.newBuilder()
                .setEmail("alice@example.com")
                .setPassword("correct-horse-battery-staple")
                .setLat(33.4484)
                .setLng(-112.0740)
                .build()
        )
        assertTrue(resp.token.isNotBlank())
        assertEquals(h.now.plusSeconds(3600).epochSecond, resp.expiresAt)
        // Cache populated.
        assertNotNull(h.cache.get(resp.token))
        // Kafka publisher saw exactly one ISSUED event.
        assertEquals(1, h.publisher.events.size)
        assertEquals(AuthTokenEvent.EventType.ISSUED, h.publisher.events[0].type)
    }

    @Test
    fun `authenticate wrong password maps to UNAUTHENTICATED`() = runTest {
        val h = newHarness()
        h.grpc.register(
            RegisterRequest.newBuilder()
                .setEmail("alice@example.com")
                .setPassword("correct-horse-battery-staple")
                .build()
        )
        val ex = assertFailsWith<StatusException> {
            h.grpc.authenticate(
                AuthRequest.newBuilder()
                    .setEmail("alice@example.com")
                    .setPassword("wrong-password")
                    .build()
            )
        }
        assertEquals(Status.UNAUTHENTICATED.code, ex.status.code)
        assertEquals(0, h.publisher.events.size)
    }

    // --- issueToken -------------------------------------------------------

    @Test
    fun `issueToken rejects non-UUID with INVALID_ARGUMENT`() = runTest {
        val h = newHarness()
        val ex = assertFailsWith<StatusException> {
            h.grpc.issueToken(IssueTokenRequest.newBuilder().setUserId("not-a-uuid").build())
        }
        assertEquals(Status.INVALID_ARGUMENT.code, ex.status.code)
    }

    @Test
    fun `issueToken for unknown user maps to UNAUTHENTICATED`() = runTest {
        val h = newHarness()
        val ex = assertFailsWith<StatusException> {
            h.grpc.issueToken(IssueTokenRequest.newBuilder().setUserId(UUID.randomUUID().toString()).build())
        }
        assertEquals(Status.UNAUTHENTICATED.code, ex.status.code)
    }

    // --- validateToken ---------------------------------------------------

    @Test
    fun `validateToken returns claims for a fresh token`() = runTest {
        val h = newHarness()
        val regResp = h.grpc.register(
            RegisterRequest.newBuilder()
                .setEmail("alice@example.com")
                .setPassword("correct-horse-battery-staple")
                .build()
        )
        val authResp = h.grpc.authenticate(
            AuthRequest.newBuilder()
                .setEmail("alice@example.com")
                .setPassword("correct-horse-battery-staple")
                .build()
        )
        val claims = h.grpc.validateToken(ValidateTokenRequest.newBuilder().setToken(authResp.token).build())
        assertEquals(regResp.userId, claims.userId)
        assertEquals(h.now.plusSeconds(3600).epochSecond, claims.expiresAt)
    }

    @Test
    fun `validateToken rejects revoked session with PERMISSION_DENIED`() = runTest {
        val h = newHarness()
        h.grpc.register(
            RegisterRequest.newBuilder()
                .setEmail("alice@example.com")
                .setPassword("correct-horse-battery-staple")
                .build()
        )
        val authResp = h.grpc.authenticate(
            AuthRequest.newBuilder()
                .setEmail("alice@example.com")
                .setPassword("correct-horse-battery-staple")
                .build()
        )
        h.grpc.revokeToken(RevokeTokenRequest.newBuilder().setToken(authResp.token).build())
        // Cache eviction by revoke means the second validate hits the DB and
        // sees the revoked flag.
        val ex = assertFailsWith<StatusException> {
            h.grpc.validateToken(ValidateTokenRequest.newBuilder().setToken(authResp.token).build())
        }
        assertEquals(Status.PERMISSION_DENIED.code, ex.status.code)
    }

    @Test
    fun `validateToken rejects garbage token with UNAUTHENTICATED`() = runTest {
        val h = newHarness()
        val ex = assertFailsWith<StatusException> {
            h.grpc.validateToken(ValidateTokenRequest.newBuilder().setToken("not.a.jwt").build())
        }
        assertEquals(Status.UNAUTHENTICATED.code, ex.status.code)
    }

    // --- revokeToken ------------------------------------------------------

    @Test
    fun `revokeToken on valid token returns success and publishes REVOKED`() = runTest {
        val h = newHarness()
        h.grpc.register(
            RegisterRequest.newBuilder()
                .setEmail("alice@example.com")
                .setPassword("correct-horse-battery-staple")
                .build()
        )
        val authResp = h.grpc.authenticate(
            AuthRequest.newBuilder()
                .setEmail("alice@example.com")
                .setPassword("correct-horse-battery-staple")
                .build()
        )
        val revoke = h.grpc.revokeToken(RevokeTokenRequest.newBuilder().setToken(authResp.token).build())
        assertTrue(revoke.success)
        val revokedEvents = h.publisher.events.filter { it.type == AuthTokenEvent.EventType.REVOKED }
        assertEquals(1, revokedEvents.size)
    }

    @Test
    fun `revokeToken on garbage token returns success=false without throwing`() = runTest {
        val h = newHarness()
        val resp = h.grpc.revokeToken(RevokeTokenRequest.newBuilder().setToken("not.a.jwt").build())
        assertFalse(resp.success)
        assertTrue(h.publisher.events.isEmpty())
    }
}
