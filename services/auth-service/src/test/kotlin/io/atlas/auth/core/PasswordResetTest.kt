package io.atlas.auth.core

import io.atlas.auth.crypto.BcryptPasswordHasher
import io.atlas.auth.crypto.Jose4jJwtSigner
import io.atlas.auth.memory.InMemorySessionRepository
import io.atlas.auth.memory.InMemoryUserRepository
import io.atlas.auth.memory.InMemoryVerificationTokenRepository
import org.junit.jupiter.api.Test
import java.time.Clock
import java.time.Duration
import java.time.Instant
import java.time.ZoneOffset
import java.util.UUID
import kotlin.test.assertEquals
import kotlin.test.assertFailsWith
import kotlin.test.assertNotNull
import kotlin.test.assertNull
import kotlin.test.assertTrue

/**
 * Password reset and email verification.
 *
 * The security-relevant behaviours here are mostly about what the system
 * REFUSES to say or do, which is why several of these assert on the
 * absence of something.
 */
class PasswordResetTest {

    private val projectA: UUID = UUID.fromString("11111111-1111-1111-1111-111111111111")
    private val projectB: UUID = UUID.fromString("22222222-2222-2222-2222-222222222222")

    /** Captures what would have been mailed, so tests can read the token. */
    private class RecordingEmailSender : EmailSender {
        val sent = mutableListOf<EmailMessage>()
        override fun send(message: EmailMessage) {
            sent += message
        }
    }

    private class Harness(
        val service: AuthService,
        val users: InMemoryUserRepository,
        val sessions: InMemorySessionRepository,
        val tokens: InMemoryVerificationTokenRepository,
        val email: RecordingEmailSender,
        val now: Instant,
    )

    private fun harness(
        now: Instant = Instant.now().truncatedTo(java.time.temporal.ChronoUnit.SECONDS),
        resetLifetime: Duration = Duration.ofHours(1),
    ): Harness {
        val clock = Clock.fixed(now, ZoneOffset.UTC)
        val users = InMemoryUserRepository(clock = clock)
        val sessions = InMemorySessionRepository()
        val tokens = InMemoryVerificationTokenRepository()
        val email = RecordingEmailSender()
        val service = AuthService(
            users = users,
            sessions = sessions,
            hasher = BcryptPasswordHasher(cost = 4),
            signer = Jose4jJwtSigner("a-very-secret-secret-of-sufficient-length"),
            clock = clock,
            verificationTokens = tokens,
            email = email,
            resetTokenLifetime = resetLifetime,
        )
        return Harness(service, users, sessions, tokens, email, now)
    }

    /** The token is in the email body and nowhere else. */
    private fun tokenFrom(message: EmailMessage): String =
        Regex("[0-9a-f]{64}").find(message.body)?.value
            ?: error("no token in message body: ${message.body}")

    // --- the happy path ---------------------------------------------------

    @Test
    fun `a reset lets the user log in with the new password`() {
        val h = harness()
        h.service.register(projectA, "alice@example.com", "the-old-password")

        h.service.requestPasswordReset(projectA, "alice@example.com")
        val token = tokenFrom(h.email.sent.single())

        h.service.resetPassword(token, "the-new-password")

        h.service.authenticate(projectA, "alice@example.com", "the-new-password")
        assertFailsWith<AuthError.InvalidCredentials> {
            h.service.authenticate(projectA, "alice@example.com", "the-old-password")
        }
    }

    // --- what it refuses to tell you --------------------------------------

    /**
     * The enumeration property. An endpoint that behaved differently for a
     * registered address would let anyone test a list of addresses and
     * learn which have accounts — and those are the ones worth attacking.
     */
    @Test
    fun `requesting a reset for an unknown address is silent`() {
        val h = harness()
        h.service.requestPasswordReset(projectA, "nobody@example.com")
        assertTrue(h.email.sent.isEmpty(), "no mail to an address with no account")
        // And no exception: the caller cannot distinguish this from success.
    }

    /** Same address, different project: not this project's user. */
    @Test
    fun `a reset request does not reach another project's user`() {
        val h = harness()
        h.service.register(projectA, "alice@example.com", "the-old-password")
        h.service.requestPasswordReset(projectB, "alice@example.com")
        assertTrue(h.email.sent.isEmpty())
    }

    // --- single use --------------------------------------------------------

    @Test
    fun `a reset token cannot be used twice`() {
        val h = harness()
        h.service.register(projectA, "alice@example.com", "the-old-password")
        h.service.requestPasswordReset(projectA, "alice@example.com")
        val token = tokenFrom(h.email.sent.single())

        h.service.resetPassword(token, "the-new-password")
        assertFailsWith<AuthError.TokenInvalid> {
            h.service.resetPassword(token, "a-third-password")
        }
        // The first reset still stands.
        h.service.authenticate(projectA, "alice@example.com", "the-new-password")
    }

    /**
     * Issuing a new link must kill the old one. Otherwise a user who
     * clicks "resend" three times leaves three live ways into their
     * account sitting in a mailbox.
     */
    @Test
    fun `requesting a second link invalidates the first`() {
        val h = harness()
        h.service.register(projectA, "alice@example.com", "the-old-password")

        h.service.requestPasswordReset(projectA, "alice@example.com")
        val first = tokenFrom(h.email.sent[0])
        h.service.requestPasswordReset(projectA, "alice@example.com")
        val second = tokenFrom(h.email.sent[1])

        assertFailsWith<AuthError.TokenInvalid> {
            h.service.resetPassword(first, "from-the-old-link")
        }
        h.service.resetPassword(second, "from-the-new-link")
    }

    @Test
    fun `an expired reset token is refused`() {
        val h = harness(resetLifetime = Duration.ofHours(1))
        h.service.register(projectA, "alice@example.com", "the-old-password")
        h.service.requestPasswordReset(projectA, "alice@example.com")
        val token = tokenFrom(h.email.sent.single())

        // A service whose clock is two hours ahead, sharing the same store.
        val later = AuthService(
            users = h.users,
            sessions = h.sessions,
            hasher = BcryptPasswordHasher(cost = 4),
            signer = Jose4jJwtSigner("a-very-secret-secret-of-sufficient-length"),
            clock = Clock.fixed(h.now.plus(Duration.ofHours(2)), ZoneOffset.UTC),
            verificationTokens = h.tokens,
            email = h.email,
        )
        assertFailsWith<AuthError.TokenInvalid> {
            later.resetPassword(token, "too-late-for-this")
        }
    }

    @Test
    fun `an unknown token is refused`() {
        val h = harness()
        assertFailsWith<AuthError.TokenInvalid> {
            h.service.resetPassword("f".repeat(64), "some-new-password")
        }
    }

    /**
     * A verification token must not double as a reset token. They are
     * mailed under different pretexts, and "confirm your address" is far
     * easier to get someone to click than "reset your password".
     */
    @Test
    fun `a verification token cannot reset a password`() {
        val h = harness()
        h.service.register(projectA, "alice@example.com", "the-old-password")
        h.service.requestEmailVerification(projectA, "alice@example.com")
        val token = tokenFrom(h.email.sent.single())

        assertFailsWith<AuthError.TokenInvalid> {
            h.service.resetPassword(token, "wrong-token-type")
        }
    }

    // --- sessions ----------------------------------------------------------

    /**
     * A reset is a statement that the old credential may be compromised.
     * Leaving sessions alive would mean an attacker who logged in before
     * the reset stays logged in after it — the user does everything right
     * and is still not safe.
     */
    @Test
    fun `a reset revokes every existing session`() {
        val h = harness()
        h.service.register(projectA, "alice@example.com", "the-old-password")
        val a = h.service.authenticate(projectA, "alice@example.com", "the-old-password")
        val b = h.service.authenticate(projectA, "alice@example.com", "the-old-password")

        val signer = Jose4jJwtSigner("a-very-secret-secret-of-sufficient-length")
        val sessionA = signer.verify(a.token).sessionId
        val sessionB = signer.verify(b.token).sessionId
        assertTrue(!h.sessions.findById(sessionA)!!.revoked)

        h.service.requestPasswordReset(projectA, "alice@example.com")
        h.service.resetPassword(tokenFrom(h.email.sent.single()), "the-new-password")

        assertTrue(h.sessions.findById(sessionA)!!.revoked, "old sessions must not survive")
        assertTrue(h.sessions.findById(sessionB)!!.revoked)
    }

    @Test
    fun `a reset rejects a password that fails the strength rules`() {
        val h = harness()
        h.service.register(projectA, "alice@example.com", "the-old-password")
        h.service.requestPasswordReset(projectA, "alice@example.com")
        val token = tokenFrom(h.email.sent.single())

        assertFailsWith<AuthError.WeakPassword> { h.service.resetPassword(token, "short") }
        // And the token survives, so the user can try again rather than
        // having to request a whole new link over a typo.
        h.service.resetPassword(token, "a-long-enough-password")
    }

    // --- email verification ------------------------------------------------

    @Test
    fun `verifying stamps the user and is idempotent`() {
        val h = harness()
        val id = h.service.register(projectA, "alice@example.com", "the-old-password")
        assertNull(h.users.findById(projectA, id)!!.emailVerifiedAt, "starts unverified")

        h.service.requestEmailVerification(projectA, "alice@example.com")
        val token = tokenFrom(h.email.sent.single())
        assertEquals(id, h.service.verifyEmail(token))

        val verifiedAt = assertNotNull(h.users.findById(projectA, id)!!.emailVerifiedAt)

        // A second click on the same link is refused as a used token, and
        // the recorded time does not move.
        assertFailsWith<AuthError.TokenInvalid> { h.service.verifyEmail(token) }
        assertEquals(verifiedAt, h.users.findById(projectA, id)!!.emailVerifiedAt)
    }

    /** No point mailing a link that would do nothing. */
    @Test
    fun `requesting verification for an already-verified address is silent`() {
        val h = harness()
        h.service.register(projectA, "alice@example.com", "the-old-password")
        h.service.requestEmailVerification(projectA, "alice@example.com")
        h.service.verifyEmail(tokenFrom(h.email.sent.single()))

        h.service.requestEmailVerification(projectA, "alice@example.com")
        assertEquals(1, h.email.sent.size, "no second mail once verified")
    }

    /**
     * Verifying an address is not evidence a password leaked, so unlike a
     * reset it must not log the user out of everything.
     */
    @Test
    fun `verifying does not revoke sessions`() {
        val h = harness()
        h.service.register(projectA, "alice@example.com", "the-old-password")
        val signed = h.service.authenticate(projectA, "alice@example.com", "the-old-password")
        val sessionId = Jose4jJwtSigner("a-very-secret-secret-of-sufficient-length")
            .verify(signed.token).sessionId

        h.service.requestEmailVerification(projectA, "alice@example.com")
        h.service.verifyEmail(tokenFrom(h.email.sent.single()))

        assertTrue(!h.sessions.findById(sessionId)!!.revoked)
    }

    /** A reset token must not confirm an address either — both directions. */
    @Test
    fun `a reset token cannot verify an email`() {
        val h = harness()
        h.service.register(projectA, "alice@example.com", "the-old-password")
        h.service.requestPasswordReset(projectA, "alice@example.com")
        assertFailsWith<AuthError.TokenInvalid> {
            h.service.verifyEmail(tokenFrom(h.email.sent.single()))
        }
    }

    // --- the token itself ---------------------------------------------------

    /**
     * Only the hash is stored. A database dump, a backup, or a stray log
     * line must not hand someone the ability to take over an account.
     */
    @Test
    fun `the plaintext token is never stored`() {
        val h = harness()
        h.service.register(projectA, "alice@example.com", "the-old-password")
        h.service.requestPasswordReset(projectA, "alice@example.com")
        val token = tokenFrom(h.email.sent.single())

        // The repository is keyed by hash; the plaintext finds nothing.
        assertNull(h.tokens.findByHash(token), "the raw token must not be a key")
        assertNotNull(h.tokens.findByHash(VerificationTokens.hash(token)))
    }

    @Test
    fun `tokens are unpredictable and long`() {
        val seen = (1..200).map { VerificationTokens.generate() }.toSet()
        assertEquals(200, seen.size, "generated tokens must not repeat")
        assertTrue(seen.all { it.length == 64 }, "32 bytes, hex-encoded")
    }

    // --- no provider configured ---------------------------------------------

    /**
     * A deployment with no mail provider must still serve login and
     * registration. Only the flows that actually need email fail, and they
     * fail by naming the missing configuration rather than by accepting
     * the request and dropping it — which a user experiences as "the email
     * never arrived" and an operator sees as nothing at all.
     */
    @Test
    fun `without an email provider only the email flows fail`() {
        val clock = Clock.fixed(Instant.now(), ZoneOffset.UTC)
        val users = InMemoryUserRepository(clock = clock)
        val service = AuthService(
            users = users,
            sessions = InMemorySessionRepository(),
            hasher = BcryptPasswordHasher(cost = 4),
            signer = Jose4jJwtSigner("a-very-secret-secret-of-sufficient-length"),
            clock = clock,
            // No verificationTokens, no email — the default wiring.
        )

        // The ordinary flows are untouched.
        service.register(projectA, "alice@example.com", "a-good-password")
        service.authenticate(projectA, "alice@example.com", "a-good-password")

        assertFailsWith<AuthError.EmailNotConfigured> {
            service.requestPasswordReset(projectA, "alice@example.com")
        }
        assertFailsWith<AuthError.EmailNotConfigured> {
            service.resetPassword("f".repeat(64), "another-password")
        }
        assertFailsWith<AuthError.EmailNotConfigured> {
            service.requestEmailVerification(projectA, "alice@example.com")
        }
        assertFailsWith<AuthError.EmailNotConfigured> {
            service.verifyEmail("f".repeat(64))
        }
    }

    // --- getUser ------------------------------------------------------------

    @Test
    fun `getUser reports verification state and is project scoped`() {
        val h = harness()
        val id = h.service.register(projectA, "alice@example.com", "a-good-password")

        val before = h.service.getUser(projectA, id)
        assertEquals("alice@example.com", before.email)
        assertNull(before.emailVerifiedAt, "starts unverified")

        h.service.requestEmailVerification(projectA, "alice@example.com")
        h.service.verifyEmail(tokenFrom(h.email.sent.single()))

        assertNotNull(h.service.getUser(projectA, id).emailVerifiedAt)

        // A real user id, wrong project: must not resolve.
        assertFailsWith<AuthError.InvalidCredentials> { h.service.getUser(projectB, id) }
    }
}
