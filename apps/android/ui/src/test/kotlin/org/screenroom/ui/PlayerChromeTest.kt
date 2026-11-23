@file:OptIn(kotlinx.coroutines.ExperimentalCoroutinesApi::class)

package org.screenroom.ui

import kotlinx.coroutines.cancelAndJoin
import kotlinx.coroutines.launch
import kotlinx.coroutines.test.*
import org.junit.Assert.*
import org.junit.Test

class PlayerChromeTest {
    @Test fun playingIdleHidesAtThreeSeconds() = runTest {
        val chrome = PlayerChrome()
        val timer = launch { chrome.idle(true, false) }
        runCurrent(); advanceTimeBy(2999); runCurrent()
        assertTrue(chrome.visible)
        advanceTimeBy(1); runCurrent()
        assertFalse(chrome.visible)
        timer.join()
    }

    @Test fun pausedBlockedPreparingAndErrorStatesNeverHide() = runTest {
        for ((playing, blocked) in listOf(false to false, false to true, true to true)) {
            val chrome = PlayerChrome()
            chrome.idle(playing, blocked)
            assertTrue(chrome.visible)
        }
        assertEquals(0L, testScheduler.currentTime)
    }

    @Test fun initialProgrammaticTvFocusAllowsIdleAndFirstSelectOnlyReveals() = runTest {
        val chrome = PlayerChrome()
        chrome.focus("toolbar", true)
        chrome.idle(true, false)
        assertFalse(chrome.visible)
        assertTrue(chrome.key(navigation = false))
        assertTrue(chrome.visible)
        assertFalse(chrome.keyboardEngaged)
        assertFalse(chrome.key(navigation = false))
    }

    @Test fun intentionalKeyboardNavigationHoldsFocusButPointerClickDoesNotLatch() = runTest {
        val chrome = PlayerChrome()
        chrome.focus("toolbar", true)
        chrome.key(navigation = true)
        chrome.idle(true, false)
        assertTrue(chrome.visible)
        assertEquals(0L, testScheduler.currentTime)
        chrome.pointer()
        chrome.idle(true, false)
        assertFalse(chrome.visible)
        assertEquals(3000L, testScheduler.currentTime)
    }

    @Test fun firstDpadRevealsWithoutNavigationThenNextDpadEngagesControls() = runTest {
        val chrome = PlayerChrome()
        chrome.idle(true, false)
        assertTrue(chrome.key(navigation = true))
        assertFalse(chrome.keyboardEngaged)
        assertFalse(chrome.key(navigation = true))
        assertTrue(chrome.keyboardEngaged)
    }

    @Test fun menuAndScrubPinChromeAndDismissBeforeFullscreenBack() = runTest {
        val chrome = PlayerChrome()
        val presentation = PlaybackPresentation().apply { fullscreen = true }
        chrome.open(PlayerMenu.TRACKS)
        chrome.idle(true, false)
        assertTrue(chrome.visible)
        assertEquals(0L, testScheduler.currentTime)
        assertTrue(chrome.dismiss())
        assertTrue(presentation.fullscreen)
        assertFalse(chrome.dismiss())
        assertTrue(presentation.exitFullscreen())
        chrome.scrubbing = true
        chrome.idle(true, false)
        assertTrue(chrome.visible)
        assertEquals(0L, testScheduler.currentTime)
        chrome.scrubbing = false
        chrome.idle(true, false)
        assertFalse(chrome.visible)
    }

    @Test fun interactionInvalidatesOldTimerAndNewIdleGetsFullDeadline() = runTest {
        val chrome = PlayerChrome()
        val old = launch { chrome.idle(true, false) }
        runCurrent(); advanceTimeBy(2000)
        chrome.pointer()
        advanceTimeBy(1000); runCurrent()
        old.join()
        assertTrue(chrome.visible)
        val fresh = launch { chrome.idle(true, false) }
        runCurrent(); advanceTimeBy(2999); runCurrent()
        assertTrue(chrome.visible)
        advanceTimeBy(1); runCurrent()
        assertFalse(chrome.visible)
        fresh.join()
    }

    @Test fun timerCancellationOnDisposalCannotHideLater() = runTest {
        val chrome = PlayerChrome()
        val timer = launch { chrome.idle(true, false) }
        runCurrent(); advanceTimeBy(1000)
        timer.cancelAndJoin()
        advanceTimeBy(10_000); runCurrent()
        assertTrue(chrome.visible)
    }

    @Test fun focusMovingBetweenGroupsDoesNotLoseKeyboardOwnership() {
        val chrome = PlayerChrome()
        chrome.key(navigation = true)
        chrome.focus("header", true)
        chrome.focus("toolbar", true)
        chrome.focus("header", false)
        assertTrue(chrome.holds(true, false))
        chrome.focus("toolbar", false)
        assertFalse(chrome.holds(true, false))
    }

    @Test fun menuTransitionsArePresentationOnlyAndMutuallyExclusive() {
        val chrome = PlayerChrome()
        chrome.open(PlayerMenu.VOLUME)
        assertEquals(PlayerMenu.VOLUME, chrome.menu)
        chrome.open(PlayerMenu.SETTINGS)
        assertEquals(PlayerMenu.SETTINGS, chrome.menu)
        chrome.dismiss()
        assertNull(chrome.menu)
        assertTrue(chrome.visible)
    }

    @Test fun transferringIdleTvFocusToStageDoesNotImmediatelyRevealAgain() = runTest {
        val chrome = PlayerChrome()
        chrome.focus("toolbar", true)
        chrome.idle(true, false)
        assertFalse(chrome.visible)
        chrome.focus("toolbar", false)
        val restarted = launch { chrome.idle(true, false) }
        runCurrent()
        assertFalse(chrome.visible)
        advanceTimeBy(3000); runCurrent()
        assertFalse(chrome.visible)
        restarted.join()
        chrome.idle(false, false)
        assertTrue(chrome.visible)
    }

    @Test fun humanNavigationThenSelectOrOtherKeyRetainsVisibleFocusedControls() = runTest {
        val chrome = PlayerChrome()
        chrome.focus("toolbar", true)
        chrome.key(navigation = true)
        assertFalse(chrome.key(navigation = false))
        assertTrue(chrome.keyboardEngaged)
        chrome.idle(true, false)
        assertTrue(chrome.visible)
        assertEquals(0L, testScheduler.currentTime)
        chrome.key(navigation = false)
        chrome.idle(true, false)
        assertTrue(chrome.visible)
        chrome.pointer()
        chrome.idle(true, false)
        assertFalse(chrome.visible)
    }

    @Test fun compactToolbarFits320DpWithoutScrollingAndWideTvKeepsLargerTargets() {
        for (television in listOf(false, true)) {
            for (width in listOf(320, 360, 390, 400, 480, 720, 1280)) {
                val padding = if (television && width >= 400) 56 else 24
                val available = (width - padding).toFloat()
                val layout = playerToolbarLayout(available, television)
                val count = if (layout.separateTracks) 7 else 6
                assertTrue("$width dp, TV=$television", count * layout.targetDp <= available)
                assertTrue(layout.targetDp >= 48)
            }
        }
        assertFalse(playerToolbarLayout(296f, false).separateTracks)
        assertEquals(56, playerToolbarLayout(1000f, true).targetDp)
        assertTrue(playerToolbarLayout(1000f, true).separateTracks)
    }

    @Test fun compactSettingsUsesSameTrackChoicesWhileFullToolbarHasDedicatedMenu() {
        assertTrue(playerMenuIncludesTracks(PlayerMenu.SETTINGS, compact = true))
        assertFalse(playerMenuIncludesTracks(PlayerMenu.SETTINGS, compact = false))
        assertTrue(playerMenuIncludesTracks(PlayerMenu.TRACKS, compact = false))
        assertTrue(playerMenuIncludesTracks(PlayerMenu.TRACKS, compact = true))
        assertFalse(playerMenuIncludesTracks(PlayerMenu.VOLUME, compact = true))
    }

    @Test fun healthyInformationStaysOffFilmButBlockedAndCleanupMessagesRemainVisible() {
        assertFalse(playerMessageOnStage("Session checked.", canControl = true))
        assertFalse(playerMessageOnStage("", canControl = false))
        for (message in listOf("Media could not be played.", "Personal playback state is unconfirmed.", "Server cleanup is pending.")) {
            assertTrue(playerMessageOnStage(message, canControl = false))
        }
    }
}
