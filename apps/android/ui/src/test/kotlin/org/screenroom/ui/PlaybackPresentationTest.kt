package org.screenroom.ui

import org.junit.Assert.*
import org.junit.Test

class PlaybackPresentationTest {
    @Test fun backLeavesFullscreenBeforeReturningToDetails() {
        val presentation = PlaybackPresentation()
        assertFalse(presentation.exitFullscreen())
        presentation.fullscreen = true
        assertTrue(presentation.exitFullscreen())
        assertFalse(presentation.fullscreen)
        assertFalse(presentation.exitFullscreen())
    }
}
