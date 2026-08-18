package io.atlas.auth.cache

import io.atlas.auth.core.TokenClaims
import org.junit.jupiter.api.Test
import java.time.Instant
import java.util.UUID
import kotlin.test.assertEquals
import kotlin.test.assertNotNull
import kotlin.test.assertNull

class TokenValidationCacheTest {

    private fun newClaims(): TokenClaims = TokenClaims(
        projectId = UUID.randomUUID(),
        userId = UUID.randomUUID(),
        sessionId = UUID.randomUUID(),
        issuedAt = Instant.now(),
        expiresAt = Instant.now().plusSeconds(3600),
        lastLat = null,
        lastLng = null,
    )

    @Test
    fun `get returns put value keyed by signature segment`() {
        val cache = TokenValidationCache()
        val token = "header.payload.signature"
        val claims = newClaims()
        cache.put(token, claims)
        assertEquals(claims, cache.get(token))
        // Same signature segment with a different header/payload still hits;
        // the signature is what makes a token unique.
        assertEquals(claims, cache.get("other-header.other-payload.signature"))
    }

    @Test
    fun `evict removes the entry`() {
        val cache = TokenValidationCache()
        val token = "h.p.sig"
        cache.put(token, newClaims())
        assertNotNull(cache.get(token))
        cache.evict(token)
        assertNull(cache.get(token))
    }

    @Test
    fun `evictByTokenHash removes only matching entries`() {
        val cache = TokenValidationCache()
        cache.putWithHash("h.p.sig1", newClaims(), "hash-A")
        cache.putWithHash("h.p.sig2", newClaims(), "hash-B")
        cache.putWithHash("h.p.sig3", newClaims(), "hash-A")
        assertEquals(3, cache.size())
        cache.evictByTokenHash("hash-A")
        assertNull(cache.get("h.p.sig1"))
        assertNull(cache.get("h.p.sig3"))
        assertNotNull(cache.get("h.p.sig2"))
    }

    @Test
    fun `signatureSegment falls back to whole token for malformed input`() {
        assertEquals("nodot", TokenValidationCache.signatureSegment("nodot"))
        assertEquals("ends.with.dot.", TokenValidationCache.signatureSegment("ends.with.dot."))
    }
}
