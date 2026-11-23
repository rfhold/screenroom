package org.screenroom.ui

import org.junit.Assert.*
import org.junit.Test

class NavigationHistoryTest {
    private data class View(val page: String, val query: String, val library: String, val filters: Set<String>, val key: String)

    @Test fun backRestoresTheSameSearchQueryScopeFiltersAndScrollIdentity() {
        val history = NavigationHistory<View>()
        val search = View("search", "Arrival", "movies", setOf("Movie"), "entry-1")
        history.push(search)
        history.push(View("detail", "", "movies", emptySet(), "entry-2"))
        assertEquals("detail", history.pop().page)
        assertSame(search, history.lastOrNull())
        assertSame(search, history.pop())
        assertFalse(history.canGoBack)
    }

    @Test fun oldViewsEvictExplicitlyAndRootClearReturnsAllRestorationKeys() {
        val history = NavigationHistory<String>(2)
        assertNull(history.push("first"))
        assertNull(history.push("second"))
        assertEquals("first", history.push("third"))
        assertEquals(listOf("second", "third"), history.clear())
        assertNull(history.lastOrNull())
        assertFalse(history.canGoBack)
    }
}
