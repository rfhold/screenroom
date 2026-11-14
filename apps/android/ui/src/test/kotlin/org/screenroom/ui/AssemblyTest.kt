package org.screenroom.ui

import kotlinx.coroutines.*
import kotlinx.coroutines.test.runTest
import org.junit.Assert.*
import org.junit.Test
import org.screenroom.core.SourceLibraryScope

class AssemblyTest {
    private data class Choice(val source: String, val library: String, val types: String, val methods: String, val subtitle: String)
    private data class Node(val id: String, val parent: String?)

    @Test fun failedWriteRetainsEveryNormalizedChoiceAndRetryWritesIt() = runTest {
        val initial = Choice("all", "", "all", "all", "off")
        val next = Choice("jellyfin", "tv", "episode", "title-only+semantic", "transcription")
        var failWrite = true
        var persisted = initial
        val state = LocalSettings(initial, { value: Choice ->
            require(value.types.isNotEmpty())
            value.copy(methods = value.methods.replace("+semantic", ""))
        }) { if (failWrite) error("false commit") else persisted = it }
        state.update(next)
        val normalized = next.copy(methods = "title-only")
        assertEquals(normalized, state.value)
        assertEquals(initial, persisted)
        assertTrue(state.warning!!.contains("not saved"))
        try { state.update(next.copy(types = "")); fail("Invalid settings accepted") } catch (_: IllegalArgumentException) { }
        assertEquals(normalized, state.value)
        failWrite = false
        state.retry()
        assertEquals(normalized, persisted)
        assertNull(state.warning)
    }

    @Test fun settingsExternalCancellationPropagates() = runTest {
        val state = LocalSettings(0, { it: Int -> it }) { awaitCancellation() }
        var canceled = false
        val job = launch { try { state.update(1) } catch (e: CancellationException) { canceled = true; throw e } }
        testScheduler.runCurrent()
        job.cancelAndJoin()
        assertTrue(canceled)
        assertNull(state.warning)
    }

    @Test fun episodeSearchWithoutCachedAncestorsLoadsFullScopedChain() = runTest {
        val scope = SourceLibraryScope("source-id", "library-id")
        val reads = mutableListOf<Pair<String, SourceLibraryScope>>()
        val episode = Node("episode", "season")
        var cache = listOf(episode)
        val ancestors = loadParentChain(episode, Node::id, Node::parent) { id ->
            reads += id to scope
            when (id) { "season" -> Node(id, "series"); else -> Node(id, null) }
        }
        cache = cache + ancestors
        val parent = cache.single { it.id == episode.parent }
        val grandparent = cache.single { it.id == parent.parent }
        assertEquals(listOf("series", "season"), listOf(grandparent.id, parent.id))
        assertEquals(listOf("season" to scope, "series" to scope), reads)
    }

    @Test fun cyclicParentsDoNotPublishPartialCache() = runTest {
        val cache = listOf(Node("old", null))
        var published = cache
        try {
            val episode = Node("episode", "season")
            val chain = loadParentChain(episode, Node::id, Node::parent) { Node(it, "episode") }
            published = listOf(episode) + chain
            fail("Cycle accepted")
        } catch (_: IllegalStateException) { }
        assertEquals(cache, published)
    }

    @Test fun deepParentsAreBoundedWithoutPublication() = runTest {
        var reads = 0
        try {
            loadParentChain(Node("0", "1"), Node::id, Node::parent) { reads++; Node(it, (it.toInt() + 1).toString()) }
            fail("Unbounded parent chain accepted")
        } catch (_: IllegalStateException) { }
        assertEquals(32, reads)
    }

    @Test fun sequentialMetadataAndCollectorsShareOneDeadlineAndRetry() = runTest {
        var canceledRead = false
        var nextRead = false
        val state = PageReadState<Int>()
        state.load { readOperation {
            delay(10_000)
            collectPages { delay(60_000); listOf(1) to "" }
            collectPages { try { delay(60_000) } finally { canceledRead = !currentCoroutineContext().isActive }; listOf(2) to "" }
            nextRead = true
            listOf(3)
        } }
        assertEquals(120_000, testScheduler.currentTime)
        assertTrue(canceledRead)
        assertFalse(nextRead)
        assertNull(state.records)
        assertEquals("Page traversal exceeded 120 seconds", state.failure)
        state.load { readOperation { collectPages { listOf(4) to "" } } }
        assertEquals(listOf(4), state.records)
        assertNull(state.failure)
    }

    @Test fun operationIncludesItemAndAncestorReadsBeforeCredits() = runTest {
        var creditsRead = false
        var published: List<Node>? = null
        try {
            readOperation {
                delay(70_000)
                val item = Node("episode", "season")
                val parents = loadParentChain(item, Node::id, Node::parent) { delay(70_000); Node(it, null) }
                creditsRead = true
                published = listOf(item) + parents
            }
            fail("Operation deadline ignored")
        } catch (_: PageTraversalTimeout) { }
        assertEquals(120_000, testScheduler.currentTime)
        assertFalse(creditsRead)
        assertNull(published)
    }

    @Test fun operationParentCancellationRemainsCancellation() = runTest {
        try {
            withTimeout(10) { readOperation { collectPages<Int> { awaitCancellation() } } }
            fail("External timeout ignored")
        } catch (_: TimeoutCancellationException) { }
    }

    @Test fun canceledOperationCannotPublishLateNonCooperativeResult() = runTest {
        var published = false
        try {
            readOperation {
                withContext(NonCancellable) { delay(120_001) }
                currentCoroutineContext().ensureActive()
                published = true
            }
            fail("Late result accepted")
        } catch (_: PageTraversalTimeout) { }
        assertFalse(published)
    }
}
