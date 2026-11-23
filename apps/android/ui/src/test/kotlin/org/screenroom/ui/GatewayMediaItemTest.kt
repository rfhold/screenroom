@file:androidx.annotation.OptIn(androidx.media3.common.util.UnstableApi::class)

package org.screenroom.ui

import androidx.media3.common.C
import androidx.media3.common.MediaItem
import androidx.media3.common.MimeTypes
import androidx.media3.common.util.Util
import org.junit.Assert.*
import org.junit.Test
import org.screenroom.core.*

class GatewayMediaItemTest {
    private val opaqueUri = "https://example.invalid/gateway/opaque-session/media"
    private fun session(mime: String) = PlaybackSession("session", "memory-only", PlaybackState.READY, null, emptyList(),
        PlaybackTransport(PlaybackMethod.DIRECT_PLAY, mime, opaqueUri, 1u, "500000", "120000", "100000"),
        1u, 0u, PlaybackReportOutcome.CONFIRMED, "", "", null, false, null)

    @Test fun vendorHlsUsesPinnedMedia3HlsClassifier() {
        for (mime in listOf("application/vnd.apple.mpegurl", "Application/Vnd.Apple.MpegURL")) {
            val before = requireNotNull(MediaItem.Builder().setUri(opaqueUri).setMimeType(mime).build().localConfiguration)
            val local = requireNotNull(gatewayMediaItem(session(mime)).localConfiguration)
            assertEquals(opaqueUri, local.uri.toString())
            assertEquals(C.CONTENT_TYPE_OTHER, Util.inferContentTypeForUriAndMimeType(before.uri, before.mimeType))
            assertEquals(MimeTypes.APPLICATION_M3U8, local.mimeType)
            assertEquals(C.CONTENT_TYPE_HLS, Util.inferContentTypeForUriAndMimeType(local.uri, local.mimeType))
            assertTrue(local.subtitleConfigurations.isEmpty())
        }
    }

    @Test fun nonVendorTypesRemainUnchanged() {
        for (mime in listOf("video/mp4", "video/MP4", "application/x-mpegURL", "application/dash+xml",
            "application/octet-stream", "unknown/type", "application/vnd.apple.mpegurl; codecs=avc1", " application/vnd.apple.mpegurl")) {
            val local = requireNotNull(gatewayMediaItem(session(mime)).localConfiguration)
            val unchanged = requireNotNull(MediaItem.Builder().setUri(opaqueUri).setMimeType(mime).build().localConfiguration)
            assertEquals(opaqueUri, local.uri.toString())
            assertEquals(unchanged.mimeType, local.mimeType)
            assertEquals(Util.inferContentTypeForUriAndMimeType(unchanged.uri, unchanged.mimeType),
                Util.inferContentTypeForUriAndMimeType(local.uri, local.mimeType))
        }
        val mp4 = requireNotNull(gatewayMediaItem(session("video/mp4")).localConfiguration)
        assertEquals(C.CONTENT_TYPE_OTHER, Util.inferContentTypeForUriAndMimeType(mp4.uri, mp4.mimeType))
    }

    @Test fun subtitleSelectionAndMetadataRemainUnchanged() {
        val selected = PlaybackTrack("sub", PlaybackTrackKind.SUBTITLE, "English", "en", true, true,
            PlaybackSubtitleMode.WEBVTT, "$opaqueUri/subtitle")
        val tracks = listOf(selected, selected.copy(id = "unselected", selected = false),
            selected.copy(id = "unavailable", available = false), selected.copy(id = "missing", mediaUrl = null))
        val local = requireNotNull(gatewayMediaItem(session("application/vnd.apple.mpegurl").copy(tracks = tracks)).localConfiguration)
        assertEquals(1, local.subtitleConfigurations.size)
        val subtitle = local.subtitleConfigurations.single()
        assertEquals(selected.mediaUrl, subtitle.uri.toString())
        assertEquals(MimeTypes.TEXT_VTT, subtitle.mimeType)
        assertEquals(selected.displayName, subtitle.label)
        assertEquals(selected.language, subtitle.language)
        assertEquals(C.SELECTION_FLAG_DEFAULT, subtitle.selectionFlags)
    }
}
