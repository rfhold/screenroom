package org.screenroom.ui

import androidx.compose.runtime.*
import kotlinx.coroutines.delay

internal enum class PlayerMenu { VOLUME, TRACKS, SETTINGS }

internal class PlayerChrome {
    var visible by mutableStateOf(true)
        private set
    var menu by mutableStateOf<PlayerMenu?>(null)
        private set
    var scrubbing by mutableStateOf(false)
    var keyboardFocused by mutableStateOf(false)
    var keyboardEngaged by mutableStateOf(false)
    var interaction by mutableLongStateOf(0)
        private set
    private val focusOwners = mutableSetOf<String>()

    fun reveal() { visible = true; interaction++ }
    fun pointer() { keyboardEngaged = false; reveal() }
    /** A hidden stage consumes the first navigation/selection event as reveal only. */
    fun key(navigation: Boolean): Boolean {
        val hidden = !visible
        reveal()
        keyboardEngaged = !hidden && (navigation || keyboardEngaged && keyboardFocused)
        return hidden
    }
    fun focus(owner: String, focused: Boolean) {
        if (focused) focusOwners.add(owner) else focusOwners.remove(owner)
        keyboardFocused = focusOwners.isNotEmpty()
    }
    fun open(value: PlayerMenu) { menu = value; reveal() }
    fun dismiss(): Boolean {
        if (menu == null) return false
        menu = null; reveal()
        return true
    }
    fun holds(playing: Boolean, blocked: Boolean) =
        !playing || blocked || menu != null || scrubbing || keyboardFocused && keyboardEngaged

    suspend fun idle(playing: Boolean, blocked: Boolean) {
        if (holds(playing, blocked)) { visible = true; return }
        val request = interaction
        delay(3000)
        if (request == interaction && !holds(playing, blocked)) visible = false
    }
}

internal data class PlayerToolbarLayout(val separateTracks: Boolean, val targetDp: Int)

internal fun playerToolbarLayout(availableWidthDp: Float, television: Boolean): PlayerToolbarLayout {
    val preferred = if (television) 56 else 48
    val separate = availableWidthDp >= preferred * 7
    val target = if (!separate && availableWidthDp < preferred * 6) 48 else preferred
    return PlayerToolbarLayout(separate, target)
}

internal fun playerMenuIncludesTracks(menu: PlayerMenu, compact: Boolean) =
    menu == PlayerMenu.TRACKS || menu == PlayerMenu.SETTINGS && compact

internal fun playerMessageOnStage(message: String, canControl: Boolean) =
    message.isNotEmpty() && !canControl
