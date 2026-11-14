package org.screenroom.ui

import kotlinx.coroutines.CancellationException
import kotlinx.coroutines.currentCoroutineContext
import kotlinx.coroutines.ensureActive

internal data class ProfileChange<T>(val link: T?, val confirmed: Boolean, val ambiguous: Boolean)

internal suspend fun checkProfileOwner(current: () -> Boolean) {
    currentCoroutineContext().ensureActive()
    if (!current()) throw CancellationException("Profile screen departed")
}

internal suspend fun <T> reconcileProfileChange(
    mutate: suspend () -> Boolean,
    invalidate: () -> Unit,
    read: suspend () -> T?,
    matches: (T?) -> Boolean,
    current: () -> Boolean = { true },
): ProfileChange<T> = readOperation {
    checkProfileOwner(current)
    var ambiguous = false
    val validReturn = try { mutate() }
    catch (e: CancellationException) { throw e }
    catch (_: Exception) { ambiguous = true; false }
    checkProfileOwner(current)
    invalidate()
    val link = read()
    checkProfileOwner(current)
    ProfileChange(link, matches(link) && (ambiguous || validReturn), ambiguous)
}
