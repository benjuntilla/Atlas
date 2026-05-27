package io.atlas.auth.crypto

import org.mindrot.jbcrypt.BCrypt

/**
 * Hashing contract. Implementations must use a salted, slow KDF (bcrypt,
 * scrypt, or argon2). Never roll your own. Never use a fast hash.
 */
interface PasswordHasher {
    fun hash(plaintext: String): String
    fun verify(plaintext: String, hash: String): Boolean
}

/**
 * bcrypt-backed [PasswordHasher].
 *
 * Cost factor 12 corresponds to ~250ms per hash on a 2024-era laptop. That
 * is intentionally slow: it prevents offline brute-force on stolen hashes
 * while staying acceptable on the auth-service request path because login
 * is a low-rate operation.
 *
 * The salt is generated per-hash and embedded in the returned string, so
 * the same plaintext hashed twice produces two different strings. Tests
 * verify this property.
 */
class BcryptPasswordHasher(private val cost: Int = 12) : PasswordHasher {
    init {
        require(cost in 4..31) { "bcrypt cost must be between 4 and 31, got $cost" }
    }

    override fun hash(plaintext: String): String =
        BCrypt.hashpw(plaintext, BCrypt.gensalt(cost))

    override fun verify(plaintext: String, hash: String): Boolean =
        try {
            BCrypt.checkpw(plaintext, hash)
        } catch (e: RuntimeException) {
            // jbcrypt throws several unchecked types for malformed input
            // (IllegalArgumentException for unknown salt versions,
            // StringIndexOutOfBoundsException for empty strings, etc.).
            // Treat all of them as verification failures rather than a
            // crash so a corrupted DB row can't take down the service.
            false
        }
}
