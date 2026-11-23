package org.screenroom.ui

import kotlinx.coroutines.test.runTest
import org.junit.Assert.*
import org.junit.Test

class EntryPagesTest {
    private suspend fun pageTwoBack(key: Any) {
        val entries = EntryPages<String>(id = { it })
        val history = NavigationHistory<String>()
        val pages = entries.get("results", key)
        pages.load { listOf("page1card") to "second" }
        pages.load { token -> assertEquals("second", token); listOf("page2card") to "third" }
        history.push("results")
        entries.get("detail", "page2card")
        val restored = entries.get(history.pop(), key)
        assertSame(pages, restored)
        assertTrue(restored.loaded)
        assertEquals(listOf("page1card", "page2card"), restored.records)
        assertEquals("third", restored.token)
        restored.load { token -> assertEquals("third", token); listOf("page3card") to "" }
        assertEquals(3, restored.records.size)
    }

    @Test fun libraryPageTwoDetailBackRetainsResultsAndContinuation() = runTest {
        pageTwoBack(listOf("library", "source", "library-id", 1))
    }
    @Test fun searchPageTwoDetailBackRetainsQueryOptionsScopeAndContinuation() = runTest {
        pageTwoBack(listOf("search", "query", setOf("Movie"), Methods(), "source", "library-id", 1))
    }
    @Test fun changedQueryScopeEpochAndEvictionNeverReuseReads() {
        val entries = EntryPages<String>(id = { it }, limit = 2)
        val first = entries.get("entry", listOf("query", "scope", 0))
        val query = entries.get("entry", listOf("different", "scope", 0))
        assertNotSame(first, query)
        val scope = entries.get("entry", listOf("different", "other", 0))
        assertNotSame(query, scope)
        val epoch = entries.get("entry", listOf("different", "other", 1))
        assertNotSame(scope, epoch)
        entries.get("second", "key"); entries.get("third", "key")
        assertNotSame(epoch, entries.get("entry", listOf("different", "other", 1)))
        entries.remove("entry")
        assertNotSame(epoch, entries.get("entry", listOf("different", "other", 1)))
    }
}
