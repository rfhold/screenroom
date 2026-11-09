package org.screenroom.ui

import kotlinx.coroutines.currentCoroutineContext
import kotlinx.coroutines.ensureActive
import kotlinx.coroutines.CancellationException
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.withContext
import org.screenroom.core.ClientException

internal suspend fun <T> coreRequest(read: suspend () -> T): T = try {
    withContext(Dispatchers.IO) { currentCoroutineContext().ensureActive(); read() }
} catch (e: ClientException.Cancelled) {
    throw CancellationException("Core request canceled", e)
}

internal suspend fun <T> collectPages(read: suspend (String) -> Pair<List<T>, String>): List<T> {
    val records = mutableListOf<T>()
    val visited = mutableSetOf<String>()
    var token = ""
    do {
        check(visited.add(token)) { "Repeated continuation token" }
        val (items, next) = read(token)
        currentCoroutineContext().ensureActive()
        records += items
        token = next
    } while (token.isNotEmpty())
    return records
}

internal class RequestGeneration {
    private var value = 0
    fun next() = ++value
    fun current(request: Int) = request == value
}
