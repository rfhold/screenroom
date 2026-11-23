package org.screenroom.ui

internal const val MAX_PLAYBACK_MS = 922_337_203_685_477L

internal fun playbackMilliseconds(value: String): Long {
    require(value.isNotEmpty() && value.all { it in '0'..'9' })
    return value.toLong().also { require(it in 0..MAX_PLAYBACK_MS) }
}

internal fun logicalPosition(playerPosition: Long, offset: Long): Long =
    Math.addExact(playerPosition.coerceAtLeast(0), offset).coerceAtMost(MAX_PLAYBACK_MS)

internal fun mediaPosition(logical: Long, offset: Long): Long = (logical - offset).coerceAtLeast(0)

internal fun playbackTime(milliseconds: Long): String {
    val seconds = milliseconds.coerceAtLeast(0) / 1000
    return if (seconds >= 3600) "%d:%02d:%02d".format(seconds / 3600, seconds / 60 % 60, seconds % 60)
    else "%d:%02d".format(seconds / 60, seconds % 60)
}

internal class PlaybackLifetime {
    var finalPosition: Long? = null
        private set
    val active get() = finalPosition == null
    fun stop(position: Long): Long {
        if (finalPosition == null) finalPosition = position
        return finalPosition!!
    }
}
