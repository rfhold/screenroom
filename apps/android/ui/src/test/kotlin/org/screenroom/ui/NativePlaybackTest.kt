@file:OptIn(kotlinx.coroutines.ExperimentalCoroutinesApi::class)
@file:androidx.annotation.OptIn(androidx.media3.common.util.UnstableApi::class)

package org.screenroom.ui

import androidx.media3.common.MediaItem
import androidx.media3.common.Player
import kotlinx.coroutines.CompletableDeferred
import kotlinx.coroutines.test.*
import org.junit.Assert.*
import org.junit.Test
import org.screenroom.core.*
import java.lang.reflect.Proxy

class NativePlaybackTest {
    private class Media {
        lateinit var listener: Player.Listener
        var position = 0L
        var playing = false
        var wanted = false
        var volume = 1f
        var releases = 0
        var installs = 0
        val player = Proxy.newProxyInstance(Player::class.java.classLoader, arrayOf(Player::class.java)) { _, method, args ->
            when (method.name) {
                "addListener" -> { listener = args!![0] as Player.Listener; null }
                "getCurrentPosition" -> position
                "isPlaying" -> playing
                "getPlayWhenReady" -> wanted
                "getPlaybackState" -> Player.STATE_READY
                "getVolume" -> volume
                "setVolume" -> { volume = args!![0] as Float; null }
                "setMediaItem" -> { installs++; position = args!![1] as Long; null }
                "setPlayWhenReady" -> { wanted = args!![0] as Boolean; null }
                "play" -> { start(); listener.onPlayWhenReadyChanged(true, Player.PLAY_WHEN_READY_CHANGE_REASON_USER_REQUEST); null }
                "pause" -> { playing = false; wanted = false; listener.onIsPlayingChanged(false); listener.onPlayWhenReadyChanged(false, Player.PLAY_WHEN_READY_CHANGE_REASON_USER_REQUEST); null }
                "seekTo" -> { position = args!![0] as Long; null }
                "stop" -> { position = 0; playing = false; null }
                "release" -> { releases++; null }
                "prepare", "clearMediaItems", "setTrackSelectionParameters", "removeListener" -> null
                else -> error("Unexpected player call ${method.name}")
            }
        } as Player
        fun start() { playing = true; wanted = true; listener.onIsPlayingChanged(true) }
    }

    private class Gateway : PlaybackGateway {
        var value = PlaybackSession("session", "memory-only", PlaybackState.READY, null, emptyList(),
            PlaybackTransport(PlaybackMethod.DIRECT_PLAY, "video/mp4", "https://example.invalid/gateway", 1u, "500000", "120000", "100000"),
            1u, 0u, PlaybackReportOutcome.CONFIRMED, "", "", null, false, null)
        val creates = mutableListOf<CreatePlaybackSessionRequest>()
        val updates = mutableListOf<UpdatePlaybackSessionRequest>()
        val reports = mutableListOf<ReportPlaybackEventRequest>()
        val stops = mutableListOf<StopPlaybackSessionRequest>()
        var failCreate = false
        var failUpdate = false
        var failReport = false
        var failStop = false
        var createGate: CompletableDeferred<Unit>? = null
        var reportGate: CompletableDeferred<Unit>? = null
        var stopGate: CompletableDeferred<Unit>? = null
        var stopValue: PlaybackSession? = null
        var missingStopReceipt = false
        var closes = 0
        var reads = 0
        override suspend fun create(request: CreatePlaybackSessionRequest): PlaybackSession {
            creates += request
            createGate?.await()
            if (failCreate) error("ambiguous")
            return value
        }
        override suspend fun get(request: GetPlaybackSessionRequest): PlaybackSession { reads++; return value }
        override suspend fun update(request: UpdatePlaybackSessionRequest): PlaybackSession {
            updates += request
            if (failUpdate) error("ambiguous")
            value = value.copy(transportRevision = 2u, transport = value.transport!!.copy(revision = 2u, startPositionMs = request.positionMs))
            return value
        }
        override suspend fun report(request: ReportPlaybackEventRequest): ReportPlaybackEventResponse {
            reports += request
            reportGate?.await()
            if (failReport) error("ambiguous")
            value = value.copy(lastAcceptedSequence = request.sequence)
            return ReportPlaybackEventResponse(request.sequence, PlaybackReportOutcome.CONFIRMED, value)
        }
        override suspend fun stop(request: StopPlaybackSessionRequest): StopPlaybackSessionResponse {
            stops += request
            stopGate?.await()
            if (failStop) error("ambiguous")
            return StopPlaybackSessionResponse(if (missingStopReceipt) null else stopValue ?: value.copy(state = PlaybackState.STOPPED))
        }
        override fun close() { closes++ }
    }

    private fun TestScope.controller(media: Media, gateway: Gateway) =
        NativePlayback(media.player, gateway, "item", SourceLibraryScope("source", "library"), StandardTestDispatcher(testScheduler)) { MediaItem.EMPTY }

    @Test fun creationRetryRetainsExactBodyAndAuthoritativeResumePolicy() = runTest {
        val gateway = Gateway().apply { failCreate = true }
        val media = Media()
        val controller = controller(media, gateway)
        runCurrent()
        assertTrue(controller.canRetryCreate)
        gateway.failCreate = false
        controller.retryCreate(); runCurrent()
        assertEquals(2, gateway.creates.size)
        assertSame(gateway.creates[0], gateway.creates[1])
        assertEquals(PlaybackStartPolicy.RESUME, gateway.creates[0].startPolicy)
        assertNull(gateway.creates[0].positionMs)
        assertEquals(20_000, media.position)
        controller.stop("exit"); runCurrent()
        assertEquals("120000", gateway.stops.single().positionMs)
        assertEquals(1u, gateway.stops.single().finalSequence)
    }

    @Test fun backgroundDuringPreparationNeverInstallsLateMediaAndCleansOwnedSession() = runTest {
        val gate = CompletableDeferred<Unit>()
        val gateway = Gateway().apply { createGate = gate }
        val media = Media()
        val controller = controller(media, gateway)
        runCurrent()
        controller.stop("background")
        gate.complete(Unit); runCurrent()
        assertEquals(0, media.installs)
        assertEquals(1, media.releases)
        assertEquals(1, gateway.closes)
        assertEquals("background", gateway.stops.single().stopReason)
        assertFalse(controller.active)
        media.start(); runCurrent()
        assertTrue(gateway.reports.isEmpty())
        assertFalse(controller.playing)
    }

    @Test fun eventsAreOrderedBySequenceNotPositionAndHeartbeatsRenewPausedSession() = runTest {
        val gateway = Gateway()
        val media = Media()
        val controller = controller(media, gateway)
        runCurrent()
        media.start(); runCurrent()
        media.position = 30_000
        controller.togglePlay(); runCurrent()
        media.position = 10_000
        controller.togglePlay(); runCurrent()
        controller.togglePlay(); runCurrent()
        advanceTimeBy(15_000); runCurrent()
        assertEquals(listOf(PlaybackEventKind.STARTED, PlaybackEventKind.PAUSED, PlaybackEventKind.RESUMED, PlaybackEventKind.PAUSED, PlaybackEventKind.HEARTBEAT), gateway.reports.map { it.kind })
        assertEquals(listOf(1u, 2u, 3u, 4u, 5u), gateway.reports.map { it.sequence })
        assertEquals(listOf("120000", "130000", "110000", "110000", "110000"), gateway.reports.map { it.positionMs })
        controller.stop("exit"); runCurrent()
    }

    @Test fun uncertainReportStopsMediaAndReadOnlyCheckDoesNotReplayIt() = runTest {
        val gateway = Gateway().apply { failReport = true }
        val media = Media()
        val controller = controller(media, gateway)
        runCurrent(); media.start(); runCurrent()
        assertFalse(controller.canControl)
        assertFalse(media.playing)
        controller.checkSession(); runCurrent()
        advanceTimeBy(30_000); runCurrent()
        assertEquals(1, gateway.reads)
        assertEquals(1, gateway.reports.size)
        controller.togglePlay()
        assertFalse(media.playing)
        controller.stop("exit"); runCurrent()
        assertEquals("120000", gateway.stops.single().positionMs)
    }

    @Test fun transportRetryAndFinalStopRetryKeepExactBodiesAndFrozenPosition() = runTest {
        val gateway = Gateway().apply { failUpdate = true; failStop = true }
        val media = Media()
        val controller = controller(media, gateway)
        runCurrent()
        media.position = 12_345
        controller.choose(null); runCurrent()
        gateway.failUpdate = false
        controller.retryTransport(); runCurrent()
        assertSame(gateway.updates[0], gateway.updates[1])
        assertTrue(gateway.updates[0].trackChoices!!.subtitlesOff)
        assertEquals("112345", gateway.updates[0].positionMs)
        controller.stop("back")
        controller.stop("disposed")
        runCurrent()
        assertEquals(1, media.releases)
        assertEquals(1, gateway.stops.size)
        assertEquals(0, gateway.closes)
        assertEquals(StopOutcome.UNCERTAIN, controller.stopOutcome)
        gateway.failStop = false
        controller.retryStop(); runCurrent()
        assertEquals(2, gateway.stops.size)
        assertSame(gateway.stops[0], gateway.stops[1])
        assertEquals("112345", gateway.stops[0].positionMs)
        assertEquals(1, gateway.closes)
    }

    @Test fun unchangedSessionCheckPreservesPauseAndPositionAndBackwardSeekRenegotiatesOnce() = runTest {
        val gateway = Gateway()
        val media = Media()
        val controller = controller(media, gateway)
        runCurrent()
        media.position = 3_000
        controller.checkSession(); runCurrent()
        assertEquals(1, media.installs)
        assertEquals(3_000, media.position)
        assertFalse(media.playing)
        controller.seekBy(-10_000); runCurrent()
        assertEquals("93000", gateway.updates.single().positionMs)
        assertEquals(2, media.installs)
        assertEquals(0, media.position)
        controller.stop("exit"); runCurrent()
    }

    @Test fun exitDuringReportFreezesFinalPositionAndRejectsLateCallbacks() = runTest {
        val gate = CompletableDeferred<Unit>()
        val gateway = Gateway().apply { reportGate = gate }
        val media = Media()
        val controller = controller(media, gateway)
        runCurrent(); media.start(); runCurrent()
        media.position = 42_000
        controller.stop("exit")
        media.start()
        gate.complete(Unit); runCurrent()
        assertFalse(controller.playing)
        assertNull(controller.session)
        assertEquals("142000", gateway.stops.single().positionMs)
        assertEquals(1, gateway.reports.size)
    }

    @Test fun playbackErrorsUseOnlySafeApplicationMessages() {
        assertTrue(playbackFailure(ClientException.Playback(PlaybackErrorCode.PROFILE_REQUIRED)).contains("personal Jellyfin profile"))
        assertFalse(playbackFailure(IllegalStateException("https://private/?cap=secret")).contains("secret"))
        assertFalse(playbackFailure(IllegalStateException("private upstream")).contains("upstream"))
    }

    @Test fun callbacksFromReplacedTransportCannotEndTheNewRevision() = runTest {
        val gateway = Gateway()
        val media = Media()
        val controller = controller(media, gateway)
        runCurrent()
        val oldListener = media.listener
        oldListener.onPlaybackStateChanged(Player.STATE_BUFFERING)
        assertTrue(controller.buffering)
        oldListener.onPlaybackStateChanged(Player.STATE_READY)
        assertFalse(controller.buffering)
        controller.choose(null); runCurrent()
        oldListener.onPlaybackStateChanged(Player.STATE_ENDED)
        oldListener.onIsPlayingChanged(true)
        runCurrent()
        assertTrue(controller.active)
        assertFalse(controller.playing)
        assertTrue(gateway.reports.isEmpty())
        controller.stop("exit"); runCurrent()
    }

    @Test fun queuedTransportNeverDispatchesAfterAnEarlierPersonalReportBecomesUncertain() = runTest {
        val gate = CompletableDeferred<Unit>()
        val gateway = Gateway().apply { reportGate = gate; failReport = true }
        val media = Media()
        val controller = controller(media, gateway)
        runCurrent(); media.start(); runCurrent()
        controller.choose(null)
        gate.complete(Unit); runCurrent()
        assertTrue(gateway.updates.isEmpty())
        assertFalse(controller.canControl)
        assertFalse(controller.canRetryTransport)
        controller.stop("exit"); runCurrent()
    }

    @Test fun absoluteSeekClampsToDurationAndAppliesLogicalOffsetOnlyOnce() = runTest {
        val gateway = Gateway()
        val media = Media()
        val controller = controller(media, gateway)
        runCurrent()
        controller.seekTo(250_123)
        assertEquals(150_123, media.position)
        controller.seekTo(MAX_PLAYBACK_MS)
        assertEquals(400_000, media.position)
        controller.stop("exit"); runCurrent()
        assertEquals("500000", gateway.stops.single().positionMs)
    }

    @Test fun cleanupPendingRetainsAuthorityUntilReadOnlyConfirmationWithoutReplayingStop() = runTest {
        val gateway = Gateway().apply { stopValue = value.copy(state = PlaybackState.STOPPED, cleanupPending = true) }
        val controller = controller(Media(), gateway)
        runCurrent(); controller.stop("background"); runCurrent()
        assertEquals(StopOutcome.PENDING, controller.stopOutcome)
        assertTrue(controller.canRecoverStop)
        assertEquals(0, gateway.closes)
        gateway.value = gateway.value.copy(state = PlaybackState.STOPPED)
        controller.checkSession(); runCurrent()
        assertEquals(StopOutcome.CONFIRMED, controller.stopOutcome)
        assertEquals(1, gateway.stops.size)
        assertEquals(1, gateway.closes)
        assertNull(controller.session)
    }

    @Test fun stoppedButUnconfirmedPersonalReportIsNotConfirmedCleanup() = runTest {
        val gateway = Gateway().apply { stopValue = value.copy(state = PlaybackState.STOPPED, reportOutcome = PlaybackReportOutcome.UNCERTAIN) }
        val controller = controller(Media(), gateway)
        runCurrent(); controller.stop("exit"); runCurrent()
        assertEquals(StopOutcome.UNCERTAIN, controller.stopOutcome)
        assertTrue(controller.canRecoverStop)
        assertFalse(controller.message.contains("cleanup confirmed"))
        advanceTimeBy(60_000); runCurrent()
        assertEquals(StopOutcome.RECOVERY_EXPIRED, controller.stopOutcome)
        assertEquals(1, gateway.closes)
        assertNull(controller.session)
        assertFalse(controller.canRecoverStop)
        controller.retryStop(); runCurrent()
        assertEquals(1, gateway.stops.size)
    }

    @Test fun outageAndMissingReceiptNeverCauseAutomaticStopOrPersonalReportReplay() = runTest {
        val gateway = Gateway().apply { failStop = true; failReport = true }
        val controller = controller(Media(), gateway)
        runCurrent(); controller.stop("disposed"); runCurrent()
        advanceTimeBy(15_000); runCurrent()
        assertEquals(1, gateway.stops.size)
        assertTrue(gateway.reports.isEmpty())
        gateway.failStop = false; gateway.missingStopReceipt = true
        controller.retryStop(); runCurrent()
        assertEquals(StopOutcome.UNCERTAIN, controller.stopOutcome)
        assertSame(gateway.stops[0], gateway.stops[1])
        advanceTimeBy(45_000); runCurrent()
        assertEquals(1, gateway.closes)
    }

    @Test fun delayedStopResponseRetainsFixedPositionAndCannotClaimEarlyConfirmation() = runTest {
        val gate = CompletableDeferred<Unit>()
        val gateway = Gateway().apply { stopGate = gate }
        val media = Media()
        val controller = controller(media, gateway)
        runCurrent(); media.position = 55_123
        controller.stop("back"); runCurrent()
        assertEquals(StopOutcome.PENDING, controller.stopOutcome)
        assertEquals(0, gateway.closes)
        assertFalse(controller.canRecoverStop)
        assertFalse(controller.message.contains("cleanup confirmed"))
        media.position = 0
        media.start()
        advanceTimeBy(10_000); runCurrent()
        assertEquals("155123", gateway.stops.single().positionMs)
        gate.complete(Unit); runCurrent()
        assertEquals(StopOutcome.CONFIRMED, controller.stopOutcome)
        assertEquals(155_123, controller.position)
        assertFalse(controller.playing)
        assertEquals(1, gateway.closes)
    }

    @Test fun fullscreenTransitionsKeepPlayerSessionAndEventSequence() = runTest {
        val gateway = Gateway()
        val media = Media()
        val controller = controller(media, gateway)
        val presentation = PlaybackPresentation()
        runCurrent(); media.start(); runCurrent()
        val session = controller.session
        presentation.fullscreen = true
        assertTrue(presentation.exitFullscreen())
        runCurrent()
        assertSame(session, controller.session)
        assertEquals(1, media.installs)
        assertEquals(0, media.releases)
        assertEquals(listOf(1u), gateway.reports.map { it.sequence })
        controller.stop("exit"); runCurrent()
    }

    @Test fun profileReplacementHidesOldRecoveryAndUnblocksPlayWithoutChangingCleanupDeadline() = runTest {
        val gateway = Gateway().apply { failStop = true }
        val controller = controller(Media(), gateway)
        val recovery = ProfileRecovery<NativePlayback>()
        val owner = recovery.capture()
        runCurrent(); controller.stop("exit"); runCurrent()
        recovery.retain(owner, controller)
        assertTrue(recovery.visible?.cleanupOpen == true)
        advanceTimeBy(20_000); runCurrent()
        recovery.replaceProfile()
        assertNull(recovery.visible)
        assertFalse(recovery.visible?.cleanupOpen == true)
        assertFalse(recovery.owns(owner, controller))
        assertTrue(controller.cleanupOpen)
        assertEquals(0, gateway.closes)
        assertEquals(1, gateway.stops.size)
        advanceTimeBy(39_999); runCurrent()
        assertEquals(0, gateway.closes)
        advanceTimeBy(1); runCurrent()
        assertEquals(StopOutcome.RECOVERY_EXPIRED, controller.stopOutcome)
        assertEquals(1, gateway.closes)
        assertEquals(1, gateway.stops.size)
        assertNull(controller.session)
        assertTrue(gateway.reports.isEmpty())
    }

    @Test fun delayedOldStopReceiptCannotReattachRecoveryToReplacementProfile() = runTest {
        val gate = CompletableDeferred<Unit>()
        val gateway = Gateway().apply { stopGate = gate; failStop = true }
        val controller = controller(Media(), gateway)
        val recovery = ProfileRecovery<NativePlayback>()
        val owner = recovery.capture()
        runCurrent(); controller.stop("exit"); runCurrent()
        recovery.retain(owner, controller)
        recovery.replaceProfile()
        advanceTimeBy(20_000); runCurrent()
        gate.complete(Unit); runCurrent()
        assertEquals(StopOutcome.UNCERTAIN, controller.stopOutcome)
        assertFalse(recovery.retain(owner, controller))
        assertNull(recovery.visible)
        assertFalse(recovery.visible?.cleanupOpen == true)
        assertEquals(0, gateway.closes)
        advanceTimeBy(40_000); runCurrent()
        assertEquals(StopOutcome.RECOVERY_EXPIRED, controller.stopOutcome)
        assertEquals(1, gateway.closes)
        assertEquals(1, gateway.stops.size)
    }
}
