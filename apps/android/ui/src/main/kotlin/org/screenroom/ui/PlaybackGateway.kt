package org.screenroom.ui

import org.screenroom.core.*

internal interface PlaybackGateway {
    suspend fun create(request: CreatePlaybackSessionRequest): PlaybackSession
    suspend fun get(request: GetPlaybackSessionRequest): PlaybackSession
    suspend fun update(request: UpdatePlaybackSessionRequest): PlaybackSession
    suspend fun report(request: ReportPlaybackEventRequest): ReportPlaybackEventResponse
    suspend fun stop(request: StopPlaybackSessionRequest): StopPlaybackSessionResponse
    fun close()
}

internal class CorePlaybackGateway(private val client: Client) : PlaybackGateway {
    override suspend fun create(request: CreatePlaybackSessionRequest) =
        requireNotNull(coreRequest { client.createPlaybackSession(request) }.session)
    override suspend fun get(request: GetPlaybackSessionRequest) =
        requireNotNull(coreRequest { client.getPlaybackSession(request) }.session)
    override suspend fun update(request: UpdatePlaybackSessionRequest) =
        requireNotNull(coreRequest { client.updatePlaybackSession(request) }.session)
    override suspend fun report(request: ReportPlaybackEventRequest) = coreRequest { client.reportPlaybackEvent(request) }
    override suspend fun stop(request: StopPlaybackSessionRequest) = coreRequest { client.stopPlaybackSession(request) }
    override fun close() { client.cancelAll(); client.close() }
}

internal fun playbackFailure(error: Exception): String = when ((error as? ClientException.Playback)?.code) {
    PlaybackErrorCode.PROFILE_REQUIRED -> "Select a personal Jellyfin profile before playback."
    PlaybackErrorCode.PROFILE_CHANGED -> "Your personal Jellyfin profile changed. Return to details before playback."
    PlaybackErrorCode.SOURCE_UNAVAILABLE -> "This source is unavailable."
    PlaybackErrorCode.ITEM_UNAVAILABLE -> "This title is unavailable for playback."
    PlaybackErrorCode.UNSUPPORTED_MEDIA -> "This media format is unsupported."
    PlaybackErrorCode.TRANSCODING_UNAVAILABLE -> "Compatible playback is unavailable on this server."
    PlaybackErrorCode.PLAYBACK_AUTHORIZATION_UNAVAILABLE -> "The server could not authorize personal playback."
    PlaybackErrorCode.PLAYBACK_CONFLICT -> "Playback conflicts with another session or source. Choose the exact library or stop the other session."
    PlaybackErrorCode.SESSION_EXPIRED -> "This playback session expired. Return to details before playback."
    PlaybackErrorCode.CAPACITY_EXHAUSTED -> "Playback capacity is full. Stop another session before playback."
    PlaybackErrorCode.OPERATION_CONFLICT -> "The playback operation conflicts with server state. Check the session."
    else -> "Playback preparation is unconfirmed. Retry uses the same request; no new session is created."
}
