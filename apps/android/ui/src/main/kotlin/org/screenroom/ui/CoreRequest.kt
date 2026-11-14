package org.screenroom.ui

import kotlinx.coroutines.CancellationException
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.currentCoroutineContext
import kotlinx.coroutines.ensureActive
import kotlinx.coroutines.withContext
import org.screenroom.core.ClientException

internal suspend fun <T> coreRequest(read: suspend () -> T): T = try {
    withContext(Dispatchers.IO) { currentCoroutineContext().ensureActive(); read().also { currentCoroutineContext().ensureActive() } }
} catch (e: ClientException.Cancelled) {
    throw CancellationException("Core request canceled", e)
}
