package org.screenroom.ui

import androidx.compose.runtime.mutableStateListOf

internal class NavigationHistory<T>(private val capacity: Int = 32) {
    private val entries = mutableStateListOf<T>()
    init { require(capacity > 0) }
    val canGoBack get() = entries.isNotEmpty()
    fun lastOrNull(): T? = entries.lastOrNull()
    fun any(predicate: (T) -> Boolean): Boolean = entries.any(predicate)
    fun push(entry: T): T? {
        entries += entry
        return if (entries.size > capacity) entries.removeAt(0) else null
    }
    fun pop(): T = entries.removeAt(entries.lastIndex)
    fun clear(): List<T> = entries.toList().also { entries.clear() }
}
