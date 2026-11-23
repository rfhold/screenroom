package org.screenroom.ui

import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.setValue

/** UI ownership only; disowning never cancels the controller's private cleanup. */
internal class ProfileRecovery<T : Any> {
    private var revision = 0L
    var visible by mutableStateOf<T?>(null)
        private set

    fun capture(): Long = revision
    fun current(owner: Long): Boolean = owner == revision
    fun owns(owner: Long, value: T): Boolean = current(owner) && visible === value
    fun retain(owner: Long, value: T): Boolean {
        if (!current(owner)) return false
        visible = value
        return true
    }
    fun replaceProfile() {
        revision++
        visible = null
    }
}
