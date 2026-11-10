package org.screenroom.ui

import kotlinx.coroutines.currentCoroutineContext
import kotlinx.coroutines.ensureActive
import kotlinx.coroutines.CancellationException
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.withContext
import kotlinx.coroutines.withTimeoutOrNull
import kotlin.coroutines.AbstractCoroutineContextElement
import kotlin.coroutines.CoroutineContext
import org.screenroom.core.ClientException
import androidx.compose.runtime.getValue
import androidx.compose.runtime.setValue
import androidx.compose.runtime.mutableStateOf

internal suspend fun <T> coreRequest(read: suspend () -> T): T = try {
    withContext(Dispatchers.IO) { currentCoroutineContext().ensureActive(); read().also { currentCoroutineContext().ensureActive() } }
} catch (e: ClientException.Cancelled) {
    throw CancellationException("Core request canceled", e)
}

class PageTraversalTimeout : Exception("Page traversal exceeded 120 seconds")

private class OperationBudget : AbstractCoroutineContextElement(Key) {
    companion object Key : CoroutineContext.Key<OperationBudget>
}
private data class OperationResult<T>(val value: T)

suspend fun <T> readOperation(read: suspend () -> T): T {
    currentCoroutineContext().ensureActive()
    if (currentCoroutineContext()[OperationBudget] != null) return read().also { currentCoroutineContext().ensureActive() }
    return (withTimeoutOrNull(120_000) {
        withContext(OperationBudget()) { OperationResult(read().also { currentCoroutineContext().ensureActive() }) }
    } ?: throw PageTraversalTimeout()).value
}

suspend fun <T> collectPages(read: suspend (String) -> Pair<List<T>, String>): List<T> = readOperation {
    val records = mutableListOf<T>()
    val visited = mutableSetOf<String>()
    var token = ""
    do {
        check(visited.add(token)) { "Repeated continuation token" }
        check(visited.size <= 512) { "Page limit exceeded" }
        val (items, next) = read(token)
        currentCoroutineContext().ensureActive()
        check(items.size <= 10_000 - records.size) { "Item limit exceeded" }
        records += items
        token = next
    } while (token.isNotEmpty())
    records
}

suspend fun <T> loadParentChain(item: T, id: (T) -> String, parent: (T) -> String?, read: suspend (String) -> T): List<T> = readOperation {
    val ancestors = mutableListOf<T>()
    val visited = mutableSetOf(id(item))
    var next = parent(item)
    while (!next.isNullOrEmpty()) {
        check(ancestors.size < 32) { "Parent depth limit exceeded" }
        check(visited.add(next)) { "Repeated parent identifier" }
        val requested = next
        val ancestor = read(requested)
        currentCoroutineContext().ensureActive()
        check(id(ancestor) == requested) { "Unexpected parent identifier" }
        ancestors += ancestor
        next = parent(ancestor)
    }
    ancestors
}

class PageReadState<T> {
    var records by mutableStateOf<List<T>?>(null)
        private set
    var failure by mutableStateOf<String?>(null)
        private set
    private val requests = RequestGeneration()

    suspend fun load(read: suspend () -> List<T>) {
        val request = requests.next()
        records = null; failure = null
        try { val result = read(); currentCoroutineContext().ensureActive(); if (requests.current(request)) records = result }
        catch (e: CancellationException) { throw e }
        catch (e: Exception) { if (requests.current(request)) failure = e.message ?: "Page read failed" }
    }
}

internal class RequestGeneration {
    private var value = 0
    fun next() = ++value
    fun current(request: Int) = request == value
}
