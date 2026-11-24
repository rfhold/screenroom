package org.screenroom.ui

import org.screenroom.core.initializeClientTelemetry
import org.screenroom.core.recordPlaybackFirstFrame

private var telemetryEnabled = false
fun initializeAppTelemetry(platform: String, environment: String) {
    telemetryEnabled = initializeClientTelemetry(platform, environment)
}

internal fun recordAppFirstFrame(elapsedNanos: Long) {
    if (telemetryEnabled) recordPlaybackFirstFrame((elapsedNanos / 1_000_000).toULong())
}
