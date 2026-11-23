package org.screenroom.ui

import org.junit.Assert.*
import org.junit.Test

class PlaybackTimelineTest {
    @Test fun decimalPositionsRetainPrecisionBeyondJavascriptIntegerSeconds() {
        val position = playbackMilliseconds("922337203685477")
        assertEquals(MAX_PLAYBACK_MS, position)
        assertEquals("922337203685477", position.toString())
        for (invalid in listOf("", "-1", "1.5", "1e3", "922337203685478")) {
            assertThrows(IllegalArgumentException::class.java) { playbackMilliseconds(invalid) }
        }
    }
    @Test fun offsetIsAppliedExactlyOnceAndBackwardSeeksRemainValid() {
        assertEquals(123_456, logicalPosition(23_456, 100_000))
        assertEquals(23_456, mediaPosition(123_456, 100_000))
        assertEquals(100_000, logicalPosition(mediaPosition(100_000, 100_000), 100_000))
        assertEquals(0, mediaPosition(50_000, 100_000))
        assertEquals(1200, logicalPosition(1200, 0))
    }
    @Test fun exitFreezesPositionAndNeverRestoresPlaybackAuthority() {
        val lifetime = PlaybackLifetime()
        assertTrue(lifetime.active)
        assertEquals(45_678, lifetime.stop(45_678))
        assertFalse(lifetime.active)
        assertEquals(45_678, lifetime.stop(0))
        assertEquals(45_678, lifetime.stop(900_000))
        assertNull(PlaybackLifetime().finalPosition)
    }
    @Test fun labelsUseMillisecondsWithoutFloatRounding() {
        assertEquals("0:00", playbackTime(999))
        assertEquals("1:01", playbackTime(61_999))
        assertEquals("1:00:00", playbackTime(3_600_000))
    }
}
