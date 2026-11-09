package org.screenroom.ui

import kotlinx.coroutines.runBlocking
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
