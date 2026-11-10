package org.screenroom.ui

import androidx.compose.runtime.*
import kotlinx.coroutines.CancellationException
import kotlinx.coroutines.currentCoroutineContext
import kotlinx.coroutines.ensureActive

class LocalSettings<T>(initial: T, private val normalize: (T) -> T, private val write: suspend (T) -> Unit) {
    var value by mutableStateOf(initial)
        private set
    var warning by mutableStateOf<String?>(null)
        private set

    suspend fun update(next: T) {
        val checked = normalize(next)
        currentCoroutineContext().ensureActive()
        value = checked
        retry()
    }

    suspend fun retry() {
        try {
            write(value)
            currentCoroutineContext().ensureActive()
            warning = null
        } catch (e: CancellationException) { throw e }
        catch (e: Exception) { warning = "Settings apply for this session but are not saved: ${e.message}" }
    }
}
