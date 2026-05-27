package io.atlas.auth.crypto

import org.junit.jupiter.api.Test
import kotlin.test.assertFalse
import kotlin.test.assertNotEquals
import kotlin.test.assertTrue

class PasswordHasherTest {

    // Cost factor 4 is bcrypt's minimum. We use it in tests because each
    // hash at cost 12 takes ~250ms, which makes a multi-test suite slow.
    private val hasher = BcryptPasswordHasher(cost = 4)

    @Test
    fun `verify returns true for correct password`() {
        val hash = hasher.hash("correct-horse-battery-staple")
        assertTrue(hasher.verify("correct-horse-battery-staple", hash))
    }

    @Test
    fun `verify returns false for wrong password`() {
        val hash = hasher.hash("correct-horse-battery-staple")
        assertFalse(hasher.verify("wrong-password", hash))
    }

    @Test
    fun `same password hashed twice produces different hashes`() {
        // Different per-hash salts mean two hashes of the same plaintext
        // must not be byte-identical.
        val a = hasher.hash("hunter2-is-a-meme-now")
        val b = hasher.hash("hunter2-is-a-meme-now")
        assertNotEquals(a, b)
        // But both verify.
        assertTrue(hasher.verify("hunter2-is-a-meme-now", a))
        assertTrue(hasher.verify("hunter2-is-a-meme-now", b))
    }

    @Test
    fun `verify returns false rather than throwing on malformed hash`() {
        assertFalse(hasher.verify("anything", "not-a-bcrypt-hash"))
        assertFalse(hasher.verify("anything", ""))
    }
}
