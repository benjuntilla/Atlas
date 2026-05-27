package io.atlas.auth.core

import io.atlas.auth.crypto.BcryptPasswordHasher
import io.atlas.auth.crypto.Jose4jJwtSigner
import io.atlas.auth.memory.InMemorySessionRepository
import io.atlas.auth.memory.InMemoryUserRepository
import org.junit.jupiter.api.Test
import java.time.Clock
import java.time.Duration
import java.time.Instant
import java.time.ZoneOffset
import kotlin.test.assertEquals
import kotlin.test.assertFailsWith
import kotlin.test.assertNotNull
import kotlin.test.assertNull
import kotlin.test.assertTrue

class AuthServiceTest {

    private fun newService(
        // Anchor to real "now" so the JwtConsumer's wall-clock expiry check
        // sees a token issued near present time. Using a hardcoded literal
        // would put `iat` in the past or future and fail verification.
        now: Instant = Instant.now().truncatedTo(java.time.temporal.ChronoUnit.SECONDS),
        tokenLifetime: Duration = Duration.ofHours(1),
    ): TestHarness {
        val users = InMemoryUserRepository(clock = Clock.fixed(now, ZoneOffset.UTC))
        val sessions = InMemorySessionRepository()
        val hasher = BcryptPasswordHasher(cost = 4)
        val signer = Jose4jJwtSigner("a-very-secret-secret-of-sufficient-length")
        val service = AuthService(
            users = users,
            sessions = sessions,
            hasher = hasher,
            signer = signer,
            tokenLifetime = tokenLifetime,
            clock = Clock.fixed(now, ZoneOffset.UTC),
        )
        return TestHarness(service, users, sessions, signer, now)
    }

    private data class TestHarness(
        val service: AuthService,
        val users: InMemoryUserRepository,
        val sessions: InMemorySessionRepository,
        val signer: Jose4jJwtSigner,
        val now: Instant,
    )

    // --- register ---------------------------------------------------------

    @Test
    fun `register creates a user with a hashed password`() {
        val h = newService()
        val id = h.service.register("alice@example.com", "correct-horse-battery-staple")
        val user = assertNotNull(h.users.findById(id))
        assertEquals("alice@example.com", user.email)
        // Password is hashed, not stored as plaintext.
        assertTrue(user.passwordHash.startsWith("\$2"))
        // Idempotency the wrong way: the same email cannot register twice.
        assertFailsWith<AuthError.EmailAlreadyExists> {
            h.service.register("alice@example.com", "another-password-here")
        }
    }

    @Test
    fun `register normalizes email casing and whitespace`() {
        val h = newService()
        h.service.register("  Alice@Example.COM  ", "correct-horse-battery-staple")
        assertNotNull(h.users.findByEmail("alice@example.com"))
        assertFailsWith<AuthError.EmailAlreadyExists> {
            h.service.register("ALICE@example.com", "another-password-here")
        }
    }

    @Test
    fun `register rejects obvious invalid emails`() {
        val h = newService()
        for (bad in listOf("", "no-at-sign", "@no-local.com", "user@nodot", "has space@x.com")) {
            assertFailsWith<AuthError.InvalidEmail>(message = "should reject '$bad'") {
                h.service.register(bad, "correct-horse-battery-staple")
            }
        }
    }

    @Test
    fun `register rejects weak passwords`() {
        val h = newService()
        assertFailsWith<AuthError.WeakPassword> { h.service.register("u@x.com", "short") }
        assertFailsWith<AuthError.WeakPassword> { h.service.register("u@x.com", " ".repeat(10)) }
        assertFailsWith<AuthError.WeakPassword> { h.service.register("u@x.com", "x".repeat(100)) }
    }

    // --- authenticate -----------------------------------------------------

    @Test
    fun `authenticate returns a token that verifies back to the same user`() {
        val h = newService()
        val userId = h.service.register("alice@example.com", "correct-horse-battery-staple")
        val token = h.service.authenticate(
            email = "alice@example.com",
            password = "correct-horse-battery-staple",
            lastLat = 33.4484,
            lastLng = -112.0740,
        )
        val claims = h.signer.verify(token.token)
        assertEquals(userId, claims.userId)
        assertEquals(33.4484, claims.lastLat)
        assertEquals(-112.0740, claims.lastLng)
        assertEquals(h.now.epochSecond, claims.issuedAt.epochSecond)
        assertEquals(h.now.plusSeconds(3600).epochSecond, claims.expiresAt.epochSecond)
    }

    @Test
    fun `authenticate omits geo claims when not provided`() {
        val h = newService()
        h.service.register("alice@example.com", "correct-horse-battery-staple")
        val token = h.service.authenticate(
            email = "alice@example.com",
            password = "correct-horse-battery-staple",
        )
        val claims = h.signer.verify(token.token)
        assertNull(claims.lastLat)
        assertNull(claims.lastLng)
    }

    @Test
    fun `authenticate rejects wrong password as InvalidCredentials`() {
        val h = newService()
        h.service.register("alice@example.com", "correct-horse-battery-staple")
        assertFailsWith<AuthError.InvalidCredentials> {
            h.service.authenticate("alice@example.com", "wrong-password")
        }
    }

    @Test
    fun `authenticate hides existence of unknown email`() {
        // The thrown error must be the same as for wrong password so the API
        // cannot be used to enumerate registered emails.
        val h = newService()
        assertFailsWith<AuthError.InvalidCredentials> {
            h.service.authenticate("never-registered@example.com", "whatever")
        }
    }

    @Test
    fun `authenticate creates a fresh session each time`() {
        val h = newService()
        h.service.register("alice@example.com", "correct-horse-battery-staple")
        val a = h.signer.verify(
            h.service.authenticate("alice@example.com", "correct-horse-battery-staple").token
        )
        val b = h.signer.verify(
            h.service.authenticate("alice@example.com", "correct-horse-battery-staple").token
        )
        assertEquals(a.userId, b.userId)
        // Two logins => two distinct session IDs.
        assertTrue(a.sessionId != b.sessionId)
    }

    // --- issueTokenForUser ------------------------------------------------

    @Test
    fun `issueTokenForUser works for known users`() {
        val h = newService()
        val id = h.service.register("alice@example.com", "correct-horse-battery-staple")
        val token = h.service.issueTokenForUser(id, lastLat = 1.0, lastLng = 2.0)
        val claims = h.signer.verify(token.token)
        assertEquals(id, claims.userId)
        assertEquals(1.0, claims.lastLat)
    }

    @Test
    fun `issueTokenForUser rejects unknown user ids`() {
        val h = newService()
        assertFailsWith<AuthError.InvalidCredentials> {
            h.service.issueTokenForUser(java.util.UUID.randomUUID())
        }
    }
}
