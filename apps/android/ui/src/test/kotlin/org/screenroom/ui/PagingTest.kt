package org.screenroom.ui

import kotlinx.coroutines.runBlocking
import kotlinx.coroutines.launch
import kotlinx.coroutines.cancelAndJoin
import kotlinx.coroutines.CancellationException
import org.screenroom.core.ClientException
import org.junit.Assert.assertEquals
import org.junit.Assert.assertFalse
import org.junit.Assert.assertTrue
import org.junit.Test

class PagingTest {
    @Test fun consumesEveryContinuation() = runBlocking {
        val seen = mutableListOf<String>()
        val items = collectPages { token ->
            seen += token
            when (token) {
                "" -> listOf(1, 2) to "second"
                "second" -> emptyList<Int>() to "third"
                else -> listOf(3) to ""
            }
        }
        assertEquals(listOf("", "second", "third"), seen)
        assertEquals(listOf(1, 2, 3), items)
    }

    @Test(expected = IllegalStateException::class)
    fun rejectsRepeatedContinuation() { runBlocking { collectPages { listOf(1) to "loop" } } }

    @Test(expected = IllegalStateException::class)
    fun rejectsTooManyPages() { runBlocking {
        var page = 0
        collectPages { emptyList<Int>() to (++page).toString() }
    } }

    @Test(expected = IllegalStateException::class)
    fun rejectsTooManyItems() { runBlocking { collectPages { List(10_001) { it } to "" } } }

    @Test fun boundsWholeTraversal() = kotlinx.coroutines.test.runTest {
        var calls = 0
        try {
            collectPages { kotlinx.coroutines.delay(60_001); listOf(++calls) to "next-$calls" }
            throw AssertionError("Traversal must time out")
        } catch (_: PageTraversalTimeout) {
            assertEquals(1, calls)
            assertEquals(120_000, testScheduler.currentTime)
        }
    }

    @Test fun parentTimeoutIsNotTraversalFailure() = kotlinx.coroutines.test.runTest {
        try {
            kotlinx.coroutines.withTimeout(10) { collectPages<Int> { kotlinx.coroutines.delay(20); emptyList<Int>() to "" } }
            throw AssertionError("Parent cancellation must propagate")
        } catch (_: kotlinx.coroutines.TimeoutCancellationException) { }
    }

    @Test fun readOwnedTimeoutIsNotTraversalFailure() = kotlinx.coroutines.test.runTest {
        try {
            collectPages<Int> { kotlinx.coroutines.withTimeout(10) { kotlinx.coroutines.delay(20); emptyList<Int>() to "" } }
            throw AssertionError("Read cancellation must propagate")
        } catch (_: kotlinx.coroutines.TimeoutCancellationException) { }
    }

    @Test fun deadlineBecomesRetryableUiStateWithoutPartialResults() = kotlinx.coroutines.test.runTest {
        val state = PageReadState<Int>()
        state.load { collectPages { token -> if (token.isEmpty()) listOf(1) to "later" else { kotlinx.coroutines.delay(120_001); listOf(2) to "" } } }
        assertEquals(null, state.records)
        assertEquals("Page traversal exceeded 120 seconds", state.failure)
        state.load { collectPages { listOf(3) to "" } }
        assertEquals(listOf(3), state.records)
        assertEquals(null, state.failure)
    }

    @Test fun externalJobCancellationDoesNotBecomeUiFailure() = kotlinx.coroutines.test.runTest {
        val state = PageReadState<Int>()
        var canceled = false
        val job = launch {
            try { state.load { collectPages<Int> { kotlinx.coroutines.awaitCancellation() } } }
            catch (e: CancellationException) { canceled = true; throw e }
        }
        testScheduler.runCurrent()
        job.cancelAndJoin()
        assertTrue(canceled)
        assertTrue(job.isCancelled)
        assertEquals(null, state.failure)
        assertEquals(null, state.records)
    }

    @Test fun rejectsStaleGeneration() {
        val generations = RequestGeneration()
        val first = generations.next()
        val second = generations.next()
        assertFalse(generations.current(first))
        assertTrue(generations.current(second))
    }

    @Test fun coreCancellationAbortsIteration() = runBlocking {
        var calls = 0
        try {
            collectPages<Int> {
                calls++
                coreRequest { throw ClientException.Cancelled() }
            }
            throw AssertionError("Cancellation must propagate")
        } catch (_: CancellationException) {
            assertEquals(1, calls)
        }
    }
}
