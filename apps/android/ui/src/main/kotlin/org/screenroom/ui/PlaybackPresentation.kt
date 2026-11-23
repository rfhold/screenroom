package org.screenroom.ui

import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.setValue

internal class PlaybackPresentation {
    var fullscreen by mutableStateOf(false)
    fun exitFullscreen(): Boolean {
        if (!fullscreen) return false
        fullscreen = false
        return true
    }
}
