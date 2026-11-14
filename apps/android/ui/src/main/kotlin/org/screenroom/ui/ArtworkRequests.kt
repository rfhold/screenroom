package org.screenroom.ui

import kotlinx.coroutines.*
import kotlinx.coroutines.sync.Mutex
import kotlinx.coroutines.sync.Semaphore
import kotlinx.coroutines.sync.withLock
import kotlinx.coroutines.sync.withPermit

internal class ArtworkRequests<T>(private val read: suspend (String) -> T) {
    private val scope = CoroutineScope(SupervisorJob() + Dispatchers.IO)
    private val permits = Semaphore(8)
    private val lock = Mutex()
    private class Entry<T>(val task: Deferred<T>, var readers: Int = 0)
    private val pending = mutableMapOf<String, Entry<T>>()
    private val cache = LinkedHashMap<String, T>()

    suspend fun load(url: String): T {
        val entry = lock.withLock {
            cache.remove(url)?.let { cached -> cache[url] = cached; return cached }
            pending.getOrPut(url) { Entry(scope.async(start = CoroutineStart.UNDISPATCHED) {
                permits.withPermit { withContext(Dispatchers.IO) { read(url) } }
            }) }.also { it.readers++ }
        }
        try {
            val value = entry.task.await()
            lock.withLock {
                cache[url] = value
                while (cache.size > 32) cache.remove(cache.keys.first())
            }
            return value
        } finally {
            withContext(NonCancellable) { lock.withLock {
                entry.readers--
                if (entry.readers == 0) { if (pending[url] === entry) pending.remove(url); entry.task.cancel() }
            } }
        }
    }

    fun close() { scope.cancel() }
}
