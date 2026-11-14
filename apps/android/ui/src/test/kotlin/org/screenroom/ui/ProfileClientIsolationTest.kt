package org.screenroom.ui

import kotlinx.coroutines.*
import org.junit.Assert.*
import org.junit.Test

class ProfileClientIsolationTest {
    private data class Config(val endpoint: String, val user: String, val timeout: UInt)
    private class Server(var link: String? = null, var writes: Int = 0)
    private class BarrierClient(val config: Config, private val server: Server) {
        private val reads = mutableSetOf<Job>()
        var invalidations = 0
        var linkReads = 0
        fun invalidateReads() {
            invalidations++
            reads.toList().forEach { it.cancel(CancellationException("Core read invalidated")) }
        }
        fun cancelAll() = invalidateReads()
        suspend fun read(started: CompletableDeferred<Unit>, release: CompletableDeferred<Unit>): String = coroutineScope {
            val job = currentCoroutineContext()[Job]!!
            reads += job
            try { started.complete(Unit); release.await(); "Catalog ready" }
            finally { reads -= job }
        }
        suspend fun mutate(started: CompletableDeferred<Unit>, settle: CompletableDeferred<Unit>): Boolean {
            server.writes++
            invalidateReads()
            try {
                withContext(NonCancellable) { started.complete(Unit); settle.await(); server.link = "saved profile" }
                return true
            } finally {
                // Mirrors MutationBarrier::drop, before any UI ownership check.
                invalidateReads()
            }
        }
        fun getLink(): String? { linkReads++; return server.link }
    }

    private data class Outcome(val catalogCompleted: Boolean, val oldCallbacks: Int, val writes: Int,
        val profileInvalidations: Int, val catalogInvalidations: Int, val reentryLink: String?)

    private suspend fun scenario(aliasCatalogClient: Boolean): Outcome = coroutineScope {
        val config = Config("https://catalog.example", "local-user", 3000u)
        val server = Server()
        val catalog = BarrierClient(config, server)
        val profile = if (aliasCatalogClient) catalog else BarrierClient(config, server)
        assertEquals(catalog.config, profile.config)
        if (!aliasCatalogClient) assertNotSame(catalog, profile)
        var active = true
        var oldCallbacks = 0
        val mutationStarted = CompletableDeferred<Unit>()
        val mutationSettles = CompletableDeferred<Unit>()
        val mutation = launch {
            reconcileProfileChange(mutate = { profile.mutate(mutationStarted, mutationSettles) },
                invalidate = { profile.invalidateReads() }, read = { profile.getLink() },
                matches = { it == "saved profile" }, current = { active })
            oldCallbacks++
        }
        mutationStarted.await()
        active = false
        mutation.cancel()
        profile.cancelAll()
        val catalogStarted = CompletableDeferred<Unit>()
        val catalogRelease = CompletableDeferred<Unit>()
        val catalogRead = async { catalog.read(catalogStarted, catalogRelease) }
        catalogStarted.await()
        mutationSettles.complete(Unit)
        mutation.join()
        catalogRelease.complete(Unit)
        val completed = try { catalogRead.await() == "Catalog ready" } catch (_: CancellationException) { false }
        assertEquals(0, profile.linkReads)
        val reentry = BarrierClient(config, server)
        val link = reentry.getLink()
        assertEquals(1, reentry.linkReads)
        Outcome(completed, oldCallbacks, server.writes, profile.invalidations, catalog.invalidations, link)
    }

    @Test fun separateProfileClientContainsLateCoreSettlementInvalidation() = runBlocking {
        val result = scenario(aliasCatalogClient = false)
        assertTrue(result.catalogCompleted)
        assertEquals(0, result.oldCallbacks)
        assertEquals(1, result.writes)
        assertEquals(3, result.profileInvalidations)
        assertEquals(0, result.catalogInvalidations)
        assertEquals("saved profile", result.reentryLink)
    }

    @Test fun sharedClientControlAbortsNewCatalogReadDespiteUiGuard() = runBlocking {
        val result = scenario(aliasCatalogClient = true)
        assertFalse(result.catalogCompleted)
        assertEquals(0, result.oldCallbacks)
        assertEquals(1, result.writes)
        assertEquals(3, result.catalogInvalidations)
    }
}
