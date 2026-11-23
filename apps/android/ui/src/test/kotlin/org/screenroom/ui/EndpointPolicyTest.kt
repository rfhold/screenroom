package org.screenroom.ui

import org.junit.Assert.assertNotNull
import org.junit.Assert.assertNull
import org.junit.Test

class EndpointPolicyTest {
    @Test fun defaultEndpointIsAllowedInBothBuildTypes() {
        assertNull(endpointError(DEFAULT_API_ENDPOINT, true))
        assertNull(endpointError(DEFAULT_API_ENDPOINT, false))
    }

    @Test fun debugAllowsOnlyExactLoopbackHttp() {
        listOf("http://127.0.0.1:50051", "http://localhost:50062", "http://LOCALHOST:50062/").forEach { assertNull(endpointError(it, true)) }
        listOf("http://example.com", "http://127.1", "http://2130706433", "http://127.0.0.1.evil.test", "http://localhost.", "http://[::1]").forEach { assertNotNull(it, endpointError(it, true)) }
    }

    @Test fun releaseRequiresExplicitHttps() {
        assertNotNull(endpointError("http://127.0.0.1:50051", false))
        assertNotNull(endpointError("http://localhost", false))
        assertNull(endpointError("https://api.example.test:443", false))
        assertNull(endpointError("https://api.example.test", true))
    }

    @Test fun rejectsAmbiguousOrCredentialedOrigins() {
        listOf("http://localhost@evil.test", "https://name:secret@example.test", "http://evil.test@localhost", "http://localhost%40evil.test", "http://localhost\\@evil.test", "http:///localhost", "//localhost", "http://localhost:0", "http://localhost:65536", "http://localhost:-2", "http://localhost/path", "http://localhost?x=1", "http://localhost#fragment", " http://localhost", "http://local_host", "file:///localhost").forEach { endpoint ->
            assertNotNull(endpoint, endpointError(endpoint, true))
            assertNotNull(endpoint, endpointError(endpoint, false))
        }
    }
}
