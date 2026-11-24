package org.screenroom.ui

import android.content.Context
import androidx.compose.runtime.*
import androidx.media3.common.C
import androidx.media3.common.AudioAttributes
import androidx.media3.common.MediaItem
import androidx.media3.common.MimeTypes
import androidx.media3.common.PlaybackException
import androidx.media3.common.Player
import androidx.media3.common.TrackSelectionParameters
import androidx.media3.common.util.UnstableApi
import androidx.media3.exoplayer.ExoPlayer
import kotlinx.coroutines.*
import kotlinx.coroutines.channels.Channel
import org.screenroom.core.*
import java.util.UUID

internal enum class StopOutcome { NONE, LOCAL_ONLY, PENDING, UNCERTAIN, CONFIRMED, RECOVERY_EXPIRED }

/** Owns its Client until final cleanup, independently of the catalog composition. */
@UnstableApi
internal class NativePlayback(val player: Player, private val gateway: PlaybackGateway, itemId: String, scope: SourceLibraryScope,
    dispatcher: CoroutineDispatcher = Dispatchers.Main.immediate,
    private val mediaItem: (PlaybackSession) -> MediaItem = ::gatewayMediaItem) {
    constructor(context: Context, client: Client, itemId: String, scope: SourceLibraryScope) :
        this(ExoPlayer.Builder(context.applicationContext).setAudioAttributes(AudioAttributes.DEFAULT, true)
            .setHandleAudioBecomingNoisy(true).build(), CorePlaybackGateway(client), itemId, scope)
    private val work = CoroutineScope(SupervisorJob() + dispatcher)
    private val commands = Channel<suspend () -> Unit>(32)
    private val lifetime = PlaybackLifetime()
    private var generation = 0
    private var offset = 0L
    private var detachedPosition: Long? = null
    private var listener: Player.Listener? = null
    private var sequence = 0u
    private var started = false
    private val startupTime = System.nanoTime()
    private var renderedFirstFrame = false
    private var applying = false
    private var fallbackUsed = false
    private var reportBlocked = false
    private var personalUncertain = false
    private var createPending = true
    private var updateRequest: UpdatePlaybackSessionRequest? = null
    private var stopRequest: StopPlaybackSessionRequest? = null
    private var closed = false
    var stopOutcome by mutableStateOf(StopOutcome.NONE)
        private set
    val cleanupOpen get() = !active && !closed
    val canRecoverStop get() = cleanupOpen && !busy && stopRequest != null
    private val creation = CreatePlaybackSessionRequest(
        UUID.randomUUID().toString(), itemId, scope,
        PlaybackClientCapabilities(true, true, true, true, false, true),
        PlaybackStartPolicy.RESUME, null)
    var session by mutableStateOf<PlaybackSession?>(null)
        private set
    var message by mutableStateOf("Preparing playback…")
        private set
    var busy by mutableStateOf(true)
        private set
    var buffering by mutableStateOf(false)
        private set
    var failedMedia by mutableStateOf(false)
        private set
    var position by mutableLongStateOf(0)
        private set
    var playing by mutableStateOf(false)
        private set
    var paused by mutableStateOf(true)
        private set
    var volume by mutableFloatStateOf(1f)
        private set
    val active get() = lifetime.active
    val canControl get() = active && !busy && !reportBlocked && updateRequest == null && session?.transport != null && !failedMedia
    val canRetryCreate get() = createPending && !busy && active
    val canRetryTransport get() = updateRequest != null && !personalUncertain && !busy && active
    val canFallback get() = active && failedMedia && !fallbackUsed && !reportBlocked && !busy && updateRequest == null && session?.transport != null

    private inner class PlaybackListener(private val revisionGeneration: Int) : Player.Listener {
        private val current get() = active && generation == revisionGeneration
        override fun onRenderedFirstFrame() {
            if (!current || renderedFirstFrame || session?.transport == null) return
            renderedFirstFrame = true
            recordAppFirstFrame(System.nanoTime() - startupTime)
        }
        override fun onIsPlayingChanged(isPlaying: Boolean) {
            if (!current) return
            playing = isPlaying
            paused = !player.playWhenReady
            if (applying || busy || reportBlocked) return
            if (isPlaying && !started) {
                started = true
                event(PlaybackEventKind.STARTED)
            }
        }
        override fun onPlayWhenReadyChanged(playWhenReady: Boolean, reason: Int) {
            if (!current) return
            paused = !playWhenReady
            if (!applying && !busy && !reportBlocked && started && player.playbackState != Player.STATE_ENDED)
                event(if (playWhenReady) PlaybackEventKind.RESUMED else PlaybackEventKind.PAUSED)
        }
        override fun onPositionDiscontinuity(oldPosition: Player.PositionInfo, newPosition: Player.PositionInfo, reason: Int) {
            if (current && reason == Player.DISCONTINUITY_REASON_SEEK && !applying && canControl && started) event(PlaybackEventKind.SEEKED)
        }
        override fun onPlaybackStateChanged(playbackState: Int) {
            if (!current) return
            buffering = playbackState == Player.STATE_BUFFERING
            if (playbackState == Player.STATE_ENDED) stop("ended")
        }
        override fun onPlayerError(error: PlaybackException) {
            if (!current) return
            failedMedia = true
            buffering = false
            player.pause()
            message = "Media could not be played. Check the session or try compatible playback."
        }
    }

    init {
        attachListener()
        work.launch {
            for (command in commands) command()
        }
        enqueue { create() }
        work.launch {
            var ticks = 0
            while (active) {
                delay(1000)
                if (!active) break
                position = currentPosition()
                ticks++
                if (ticks % 15 == 0 && canControl && started)
                    event(if (ticks % 30 == 0 && playing) PlaybackEventKind.PROGRESS else PlaybackEventKind.HEARTBEAT)
            }
        }
    }

    private fun enqueue(action: suspend () -> Unit) {
        if (commands.trySend(action).isSuccess) return
        if (active) {
            busy = false
            block("Playback control queue is full. Stop before playback continues.", personal = true)
        } else work.launch { commands.send(action) }
    }
    private fun attachListener() {
        listener?.let(player::removeListener)
        listener = PlaybackListener(generation).also(player::addListener)
    }
    private fun currentPosition() = lifetime.finalPosition ?: detachedPosition ?: logicalPosition(player.currentPosition, offset)
    private fun safeSession(value: PlaybackSession?): PlaybackSession = requireNotNull(value) { "Missing playback session" }
    private fun healthy(value: PlaybackSession) = value.state in setOf(PlaybackState.READY, PlaybackState.PLAYING, PlaybackState.PAUSED) &&
        value.pendingOperation == null && value.reportOutcome !in setOf(PlaybackReportOutcome.UNCERTAIN, PlaybackReportOutcome.PENDING, PlaybackReportOutcome.FAILED)

    private suspend fun create() {
        if (!active || !createPending) return
        busy = true
        try {
            val result = gateway.create(creation)
            session = result
            createPending = false
            sequence = result.lastAcceptedSequence
            if (active) applySession(result, true)
        } catch (error: Exception) {
            currentCoroutineContext().ensureActive()
            if (active) message = playbackFailure(error)
        }
        finally { busy = false }
    }

    private fun applySession(value: PlaybackSession, play: Boolean) {
        if (!active) return
        session = value
        if (!healthy(value) || value.transport == null) {
            block("Playback is unavailable or its personal-state outcome is unconfirmed. Stop before starting again.")
            return
        }
        val transport = value.transport!!
        if (personalUncertain) {
            block("Personal playback state remains unconfirmed. Stop before starting again.")
            return
        }
        reportBlocked = false
        generation++
        applying = true
        try {
            attachListener()
            offset = playbackMilliseconds(transport.timelineOffsetMs)
            position = playbackMilliseconds(transport.startPositionMs)
            detachedPosition = null
            val sidecarSelected = value.tracks.any { it.selected && it.available && it.subtitleMode == PlaybackSubtitleMode.WEBVTT && it.mediaUrl != null }
            player.trackSelectionParameters = TrackSelectionParameters.Builder().setTrackTypeDisabled(C.TRACK_TYPE_TEXT, !sidecarSelected).build()
            player.setMediaItem(mediaItem(value), mediaPosition(position, offset))
            player.prepare()
            player.playWhenReady = play
            failedMedia = false
            message = ""
        } finally { applying = false }
    }

    private fun block(text: String, personal: Boolean = false) {
        if (personal) personalUncertain = true
        reportBlocked = true
        applying = true
        detachedPosition = currentPosition()
        position = detachedPosition!!
        generation++
        listener?.let(player::removeListener)
        listener = null
        player.stop()
        player.clearMediaItems()
        playing = false
        paused = true
        buffering = false
        applying = false
        message = text
    }

    private fun event(kind: PlaybackEventKind) {
        val value = session ?: return
        if (!active || reportBlocked || busy || !started) return
        if (sequence == UInt.MAX_VALUE) { block("Playback event limit reached. Stop before starting again."); return }
        sequence++
        val body = ReportPlaybackEventRequest(value.sessionId, value.controlCapability, value.transportRevision,
            sequence, kind, currentPosition().toString(), !player.playWhenReady, player.volume == 0f, player.volume.toDouble())
        val revisionGeneration = generation
        enqueue {
            if (!active || reportBlocked || generation != revisionGeneration) return@enqueue
            try {
                val response = gateway.report(body)
                val result = safeSession(response.session)
                if (active && generation == revisionGeneration) {
                    session = result
                    if (response.reportOutcome != PlaybackReportOutcome.CONFIRMED || !healthy(result))
                        block("Personal playback state is unconfirmed. Check the session; reports will not replay automatically.", personal = true)
                }
            } catch (_: Exception) {
                currentCoroutineContext().ensureActive()
                if (active) block("Playback report is unconfirmed. Check the session or stop; the event will not replay.", personal = true)
            }
        }
    }

    fun retryCreate() { if (canRetryCreate) { busy = true; enqueue { create() } } }
    fun togglePlay() { if (canControl) { if (player.playWhenReady) player.pause() else player.play() } }
    fun changePlaying(value: Boolean) { if (canControl) { if (value) player.play() else player.pause() } }
    fun changeVolume(value: Float) { if (canControl) { volume = value.coerceIn(0f, 1f); player.volume = volume } }
    fun seekBy(delta: Long) {
        if (!canControl) return
        seekTo(currentPosition() + delta)
    }
    fun seekTo(logical: Long) {
        if (!canControl) return
        val duration = session?.transport?.durationMs?.let(::playbackMilliseconds) ?: MAX_PLAYBACK_MS
        val target = logical.coerceIn(0, duration)
        if (target < offset) update(null, false, target) else player.seekTo(mediaPosition(target, offset))
    }
    fun choose(track: PlaybackTrack?) {
        val choices = when (track?.kind) {
            PlaybackTrackKind.AUDIO -> PlaybackTrackChoices(track.id, null, false)
            else -> PlaybackTrackChoices(null, track?.id, track == null)
        }
        update(choices, false, currentPosition())
    }
    fun fallback() { if (canFallback) { fallbackUsed = true; update(null, true, currentPosition()) } }
    private fun update(choices: PlaybackTrackChoices?, fallback: Boolean, target: Long) {
        val value = session ?: return
        if (!active || busy || reportBlocked || updateRequest != null) return
        updateRequest = UpdatePlaybackSessionRequest(value.sessionId, value.controlCapability, UUID.randomUUID().toString(),
            value.transportRevision, target.toString(), choices, fallback.takeIf { it })
        val resume = player.playWhenReady
        busy = true
        applying = true; player.pause(); applying = false
        enqueue { sendUpdate(resume) }
    }
    private suspend fun sendUpdate(resume: Boolean) {
        val request = updateRequest ?: return
        if (!active || personalUncertain) { busy = false; return }
        busy = true
        try {
            val result = gateway.update(request)
            session = result
            if (active) {
                updateRequest = null
                applySession(result, resume)
            }
        } catch (_: Exception) {
            currentCoroutineContext().ensureActive()
            if (active) message = "Transport change is unconfirmed. Retry preserves the exact request, or check the session."
        }
        finally { busy = false }
    }
    fun retryTransport() { if (canRetryTransport) { busy = true; enqueue { sendUpdate(false) } } }
    fun checkSession() {
        if (!active) { checkStop(); return }
        val value = session ?: return
        if (!active || busy) return
        busy = true
        enqueue {
            try {
                val result = gateway.get(GetPlaybackSessionRequest(value.sessionId, value.controlCapability))
                if (active) {
                    val unchanged = session?.transportRevision == result.transportRevision
                    session = result
                    if (reportBlocked || !healthy(result)) block("Session checked. Personal-state outcome remains unsafe to replay. Stop before starting again.")
                    else if (!unchanged) {
                        updateRequest = null
                        applySession(result, false)
                    } else message = if (updateRequest != null) "Session checked. Retry the exact transport request or stop." else "Session checked."
                }
            } catch (_: Exception) {
                currentCoroutineContext().ensureActive()
                if (active) message = "Session state could not be checked. No playback operation was replayed."
            }
            finally { busy = false }
        }
    }

    fun stop(reason: String) {
        if (!active) return
        val finalPosition = lifetime.stop(currentPosition())
        generation++
        playing = false
        paused = true
        buffering = false
        position = finalPosition
        listener?.let(player::removeListener)
        listener = null
        player.stop()
        player.clearMediaItems()
        player.release()
        busy = true
        stopOutcome = StopOutcome.LOCAL_ONLY
        message = "Media stopped on this device. Confirming server cleanup…"
        work.launch {
            delay(60_000)
            if (!closed) {
                stopOutcome = StopOutcome.RECOVERY_EXPIRED
                message = "Media stopped locally. Server cleanup is unconfirmed; recovery expired."
                closeGateway()
            }
        }
        enqueue {
                val value = session
                if (value != null) {
                    val finalSequence = if (sequence < UInt.MAX_VALUE) sequence + 1u else sequence
                    stopRequest = StopPlaybackSessionRequest(value.sessionId, value.controlCapability, UUID.randomUUID().toString(),
                        finalSequence, value.transportRevision, finalPosition.toString(), reason)
                    sendStop()
                } else {
                    message = "Media stopped locally. No server session receipt is available to confirm cleanup."
                    closeGateway()
                }
        }
    }

    private suspend fun sendStop() {
        val request = stopRequest ?: return
        busy = true
        stopOutcome = StopOutcome.PENDING
        message = "Media stopped locally. Waiting for server cleanup receipt…"
        try { acceptStop(gateway.stop(request).session) }
        catch (_: Exception) {
            currentCoroutineContext().ensureActive()
            stopOutcome = StopOutcome.UNCERTAIN
            message = "Media stopped locally. Server cleanup is unconfirmed. Retry the exact stop or check state."
        } finally { busy = false }
    }

    private fun acceptStop(value: PlaybackSession?) {
        if (closed) return
        if (value != null) session = value
        stopOutcome = when {
            value == null -> StopOutcome.UNCERTAIN
            value.cleanupPending || value.pendingOperation != null || value.reportOutcome == PlaybackReportOutcome.PENDING ||
                value.state == PlaybackState.STOPPING -> StopOutcome.PENDING
            value.state == PlaybackState.STOPPED && value.reportOutcome == PlaybackReportOutcome.CONFIRMED -> StopOutcome.CONFIRMED
            else -> StopOutcome.UNCERTAIN
        }
        message = when (stopOutcome) {
            StopOutcome.CONFIRMED -> "Playback stopped. Server cleanup confirmed."
            StopOutcome.PENDING -> "Media stopped locally. Server cleanup or personal reporting is pending."
            else -> "Media stopped locally. Server cleanup or personal reporting remains unconfirmed."
        }
        if (stopOutcome == StopOutcome.CONFIRMED) closeGateway()
    }

    fun retryStop() {
        if (!canRecoverStop) return
        busy = true
        enqueue { sendStop() }
    }

    private fun checkStop() {
        val request = stopRequest ?: return
        if (!canRecoverStop) return
        busy = true
        enqueue {
            try { acceptStop(gateway.get(GetPlaybackSessionRequest(request.sessionId, request.controlCapability))) }
            catch (_: Exception) {
                currentCoroutineContext().ensureActive()
                stopOutcome = StopOutcome.UNCERTAIN
                message = "Media stopped locally. Cleanup state could not be checked; nothing was replayed."
            } finally { busy = false }
        }
    }

    private fun closeGateway() {
        if (closed) return
        closed = true
        busy = false
        session = null
        stopRequest = null
        updateRequest = null
        gateway.close()
        commands.close()
        work.cancel()
    }
}

@UnstableApi
internal fun gatewayMediaItem(value: PlaybackSession): MediaItem {
    val transport = requireNotNull(value.transport)
    val subtitles = value.tracks.filter { it.selected && it.available && it.subtitleMode == PlaybackSubtitleMode.WEBVTT && it.mediaUrl != null }
        .map { MediaItem.SubtitleConfiguration.Builder(android.net.Uri.parse(it.mediaUrl!!))
            .setMimeType(MimeTypes.TEXT_VTT).setLabel(it.displayName).setLanguage(it.language)
            .setSelectionFlags(C.SELECTION_FLAG_DEFAULT).build() }
    val mimeType = if (transport.mimeType.equals("application/vnd.apple.mpegurl", ignoreCase = true)) MimeTypes.APPLICATION_M3U8 else transport.mimeType
    return MediaItem.Builder().setUri(transport.mediaUrl).setMimeType(mimeType).setSubtitleConfigurations(subtitles).build()
}
