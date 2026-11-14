package org.screenroom.ui

import kotlinx.coroutines.*
import org.junit.Assert.*
import org.junit.Test
import java.util.concurrent.atomic.AtomicInteger

class NativeParityTest {
    @Test fun episodeLabelsRespectOnlyExplicitSeasonContext() {
        assertEquals(CardLabels("Series", "S05:E09", "Actual title"), cardLabels("Episode", "Actual title", "Series · S05E09", 2020u))
        assertEquals(CardLabels("", "E09", "Actual title"), cardLabels("Episode", "Actual title", "Series · S5 E9 · 45m", 2020u, true))
        assertEquals("Series · S00:E100", cardLabels("Episode", "Title", "Series · S0E100", 0u).heading)
        listOf("Season 5", "Series · S-1E2", "Series · S5E", "Series · S1E2garbage").forEach {
            assertEquals(CardLabels("Episode", null, "Actual title"), cardLabels("Episode", "Actual title", it, 0u, true))
        }
    }

    @Test fun shortEpisodeLabelRequiresMatchingActualSeasonParent() {
        assertTrue(episodeSeasonContext("Episode", "season-5", "Season", "season-5"))
        listOf(
            episodeSeasonContext("Episode", "another-season", "Season", "season-5"),
            episodeSeasonContext("Episode", null, "Season", "season-5"),
            episodeSeasonContext("Episode", "series", "Series", "series"),
            episodeSeasonContext("Movie", "season-5", "Season", "season-5"),
            episodeSeasonContext("Episode", "", "Season", ""),
        ).forEach { assertFalse(it) }
        val wrongParent = episodeSeasonContext("Episode", "other", "Season", "season-5")
        assertEquals("Series · S05:E09", cardLabels("Episode", "Title", "Series · S5E9", 0u, wrongParent).heading)
    }

    @Test fun rowErrorNeverIncludesPrivateServerText() {
        assertTrue(progressError("progress_unavailable").startsWith("Continue Watching is unavailable."))
        assertEquals("This row could not be loaded. Other catalog rows remain available.", progressError("private-id"))
    }

    @Test fun incrementalPagesDisplayFirstHundredThenAppendInApiOrderAndRetainRetryToken() = runBlocking {
        val state = IncrementalPages<Int> { it.toString() }
        var calls = 0
        state.load { calls++; (100 downTo 1).toList() to "next" }
        assertEquals(1, calls)
        assertEquals(100, state.records.size)
        state.load { assertEquals("next", it); error("private server failure") }
        assertEquals(100, state.records.size)
        assertEquals("next", state.token)
        assertFalse(state.failure!!.contains("private"))
        state.load { listOf(1, 101) to "" }
        assertEquals((100 downTo 1).toList() + 101, state.records)
        assertNull(state.failure)
        state.load { error("must not read after terminal page") }
    }

    @Test fun staleAndConcurrentPageLoadsCannotOverwriteNewScope() = runBlocking {
        val state = IncrementalPages<Int> { it.toString() }
        val gate = CompletableDeferred<Unit>()
        val old = launch { state.load { gate.await(); listOf(1) to "old" } }
        yield()
        state.load { error("single flight") }
        state.load(reset = true) { listOf(2) to "fresh" }
        gate.complete(Unit); old.join()
        assertEquals(listOf(2), state.records)
        assertEquals("fresh", state.token)
        state.load { listOf(3) to "fresh" }
        assertEquals(listOf(2), state.records)
        assertNotNull(state.failure)
    }

    @Test fun pageDeadlineRetainsPreviousItems() = kotlinx.coroutines.test.runTest {
        val state = IncrementalPages<Int> { it.toString() }
        state.load { listOf(1) to "next" }
        state.load { delay(120_001); listOf(2) to "" }
        assertEquals(listOf(1), state.records)
        assertEquals("next", state.token)
        assertNotNull(state.failure)
    }

    @Test fun ambiguousProfileWriteIsIssuedOnceThenReconciledByReadOnlyRetry() = runBlocking {
        var writes = 0
        var invalidations = 0
        var reads = 0
        val result = reconcileProfileChange(mutate = { writes++; error("lost reply") }, invalidate = { invalidations++ }, read = { reads++; "profile" }, matches = { it == "profile" })
        assertEquals(ProfileChange("profile", true, true), result)
        assertEquals(1, writes); assertEquals(1, invalidations); assertEquals(1, reads)
        val rejected = reconcileProfileChange(mutate = { writes++; true }, invalidate = {}, read = { "old" }, matches = { it == "new" })
        assertFalse(rejected.confirmed)
        assertEquals(2, writes)
    }

    @Test fun failedReconciliationDoesNotRepeatTheWrite() = runBlocking {
        var writes = 0
        var reads = 0
        try {
            reconcileProfileChange(mutate = { writes++; error("lost reply") }, invalidate = {}, read = { reads++; error("read unavailable") }, matches = { _: String? -> false })
            fail("Read failure must remain unknown")
        } catch (_: IllegalStateException) { }
        suspend fun retryRead(): String { reads++; return "saved profile" }
        assertEquals("saved profile", retryRead())
        assertEquals(1, writes)
        assertEquals(2, reads)
    }

    @Test fun departedProfileMutationCannotInvalidateOrReadNewScreen() = runBlocking {
        var active = true
        var writes = 0
        var invalidations = 0
        var reads = 0
        try {
            reconcileProfileChange(mutate = { writes++; active = false; true }, invalidate = { invalidations++ },
                read = { reads++; "saved" }, matches = { it == "saved" }, current = { active })
            fail("Departed mutation must not publish")
        } catch (_: CancellationException) { }
        assertEquals(1, writes)
        assertEquals(0, invalidations)
        assertEquals(0, reads)
        active = true
        checkProfileOwner { active }
        reads++
        assertEquals(1, writes)
        assertEquals(1, reads)
    }

    @Test fun canceledProfileWriteNeverInvalidatesAfterLateCompletion() = runBlocking {
        val gate = CompletableDeferred<Unit>()
        var writes = 0
        var invalidations = 0
        val job = launch {
            reconcileProfileChange(mutate = { writes++; withContext(NonCancellable) { gate.await() }; true },
                invalidate = { invalidations++ }, read = { "saved" }, matches = { it == "saved" })
        }
        yield(); job.cancel(); gate.complete(Unit); job.join()
        assertEquals(1, writes)
        assertEquals(0, invalidations)
        assertTrue(job.isCancelled)
    }

    @Test fun artworkBoundSharingAndCancellationReleaseSlots() = runBlocking {
        val active = AtomicInteger()
        val maximum = AtomicInteger()
        val started = AtomicInteger()
        val release = CompletableDeferred<Unit>()
        val requests = ArtworkRequests { url ->
            started.incrementAndGet()
            val count = active.incrementAndGet()
            maximum.updateAndGet { maxOf(it, count) }
            try { release.await(); url } finally { active.decrementAndGet() }
        }
        try {
            val jobs = (0..19).map { index -> async { requests.load("url-$index") } }
            withTimeout(5_000) { while (started.get() < 8) yield() }
            val shared = async { requests.load("url-0") }
            yield()
            jobs[19].cancelAndJoin()
            jobs[0].cancelAndJoin()
            release.complete(Unit)
            jobs.drop(1).dropLast(1).awaitAll()
            assertEquals("url-0", shared.await())
            assertEquals(8, maximum.get())
            assertEquals(0, active.get())
            val previous = started.get()
            assertEquals("url-0", requests.load("url-0"))
            assertEquals(previous, started.get())
        } finally { requests.close() }
    }

    @Test fun cancelingRunningAndQueuedArtworkLeavesQueueUsable() = runBlocking {
        val started = AtomicInteger()
        val requests = ArtworkRequests { url -> started.incrementAndGet(); if (url != "last") awaitCancellation(); url }
        try {
            val running = (0..7).map { async { requests.load("running-$it") } }
            withTimeout(5_000) { while (started.get() < 8) yield() }
            val canceled = async { requests.load("canceled") }
            yield(); canceled.cancelAndJoin()
            val last = async { requests.load("last") }
            yield(); running.first().cancelAndJoin()
            assertEquals("last", withTimeout(5_000) { last.await() })
            assertEquals(9, started.get())
            running.drop(1).forEach { it.cancelAndJoin() }
        } finally { requests.close() }
    }
}
