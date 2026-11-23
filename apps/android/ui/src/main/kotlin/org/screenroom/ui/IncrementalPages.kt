package org.screenroom.ui

import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.setValue
import kotlinx.coroutines.CancellationException
import kotlinx.coroutines.currentCoroutineContext
import kotlinx.coroutines.ensureActive
import kotlinx.coroutines.Job
import kotlinx.coroutines.coroutineScope

internal class IncrementalPages<T>(private val id: (T) -> String) {
    var status by mutableStateOf("")
    var records by mutableStateOf(emptyList<T>())
        private set
    var token by mutableStateOf("")
        private set
    var loaded by mutableStateOf(false)
        private set
    var busy by mutableStateOf(false)
        private set
    var failure by mutableStateOf<String?>(null)
        private set
    private val requests = RequestGeneration()
    private val visited = mutableSetOf<String>()
    private var active: Job? = null

    suspend fun load(reset: Boolean = false, read: suspend (String) -> Pair<List<T>, String>): Unit = coroutineScope {
        if (!reset && (busy || (loaded && token.isEmpty()))) return@coroutineScope
        val request = requests.next()
        if (reset) active?.cancel()
        active = currentCoroutineContext()[Job]
        if (reset) { records = emptyList(); token = ""; loaded = false; visited.clear() }
        val requested = token
        busy = true; failure = null
        try {
            val (items, next) = readOperation { read(requested) }
            currentCoroutineContext().ensureActive()
            if (!requests.current(request)) return@coroutineScope
            check(visited.size < 512 && requested !in visited) { "Page limit exceeded" }
            check(next.isEmpty() || (next != requested && next !in visited)) { "Repeated continuation token" }
            val combined = (records + items).distinctBy(id)
            check(combined.size <= 10_000) { "Item limit exceeded" }
            records = combined; token = next; loaded = true; visited += requested
        } catch (e: CancellationException) { throw e }
        catch (_: Exception) { if (requests.current(request)) failure = "Unable to load items. Retry to read this page again." }
        finally { if (requests.current(request)) { busy = false; active = null } }
    }
}
