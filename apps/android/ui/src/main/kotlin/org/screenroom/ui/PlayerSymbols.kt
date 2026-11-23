package org.screenroom.ui

import androidx.compose.ui.graphics.Color
import androidx.compose.ui.graphics.SolidColor
import androidx.compose.ui.graphics.StrokeCap
import androidx.compose.ui.graphics.StrokeJoin
import androidx.compose.ui.graphics.vector.ImageVector
import androidx.compose.ui.graphics.vector.path
import androidx.compose.ui.unit.dp

internal enum class PlayerSymbol { BACK, PLAY, PAUSE, REWIND, FORWARD, VOLUME, MUTED, TRACKS, SETTINGS, FULLSCREEN, EXIT_FULLSCREEN, STOP, CHECK }

internal val playerSymbols = PlayerSymbol.entries.associateWith { symbol ->
    ImageVector.Builder(symbol.name, 24.dp, 24.dp, 24f, 24f).apply {
        path(stroke = SolidColor(Color.White), strokeLineWidth = 1.6f, strokeLineCap = StrokeCap.Round, strokeLineJoin = StrokeJoin.Round) {
            when (symbol) {
                PlayerSymbol.BACK -> { moveTo(14f, 5f); lineTo(7f, 12f); lineTo(14f, 19f) }
                PlayerSymbol.PLAY -> { moveTo(8f, 4f); lineTo(20f, 12f); lineTo(8f, 20f); close() }
                PlayerSymbol.PAUSE -> { moveTo(8f, 5f); lineTo(8f, 19f); moveTo(16f, 5f); lineTo(16f, 19f) }
                PlayerSymbol.REWIND, PlayerSymbol.FORWARD -> {
                    val reverse = symbol == PlayerSymbol.REWIND
                    val start = if (reverse) 5f else 19f
                    moveTo(start, 7f); arcTo(8f, 8f, 0f, true, !reverse, if (reverse) 4f else 20f, 15f)
                    moveTo(start, 3f); lineTo(start, 7f); lineTo(if (reverse) 9f else 15f, 7f)
                    moveTo(9f, 11f); lineTo(12f, 11f); lineTo(10f, 13f); lineTo(12f, 14f); lineTo(12f, 16f); lineTo(9f, 16f)
                    moveTo(14f, 11f); lineTo(17f, 11f); lineTo(17f, 16f); lineTo(14f, 16f); close()
                }
                PlayerSymbol.VOLUME, PlayerSymbol.MUTED -> {
                    moveTo(3f, 9f); lineTo(7f, 9f); lineTo(12f, 5f); lineTo(12f, 19f); lineTo(7f, 15f); lineTo(3f, 15f); close()
                    if (symbol == PlayerSymbol.MUTED) {
                        moveTo(17f, 9f); lineTo(22f, 15f); moveTo(22f, 9f); lineTo(17f, 15f)
                    } else {
                        moveTo(16f, 8f); curveTo(19f, 10f, 19f, 14f, 16f, 16f)
                        moveTo(19f, 5f); curveTo(24f, 9f, 24f, 15f, 19f, 19f)
                    }
                }
                PlayerSymbol.TRACKS -> {
                    moveTo(3f, 5f); lineTo(21f, 5f); lineTo(21f, 19f); lineTo(3f, 19f); close()
                    moveTo(6f, 10f); lineTo(10f, 10f); moveTo(14f, 10f); lineTo(18f, 10f)
                    moveTo(6f, 14f); lineTo(8f, 14f); moveTo(12f, 14f); lineTo(18f, 14f)
                }
                PlayerSymbol.SETTINGS -> {
                    moveTo(4f, 6f); lineTo(20f, 6f); moveTo(4f, 12f); lineTo(20f, 12f); moveTo(4f, 18f); lineTo(20f, 18f)
                    moveTo(9f, 3f); lineTo(9f, 9f); moveTo(15f, 9f); lineTo(15f, 15f); moveTo(9f, 15f); lineTo(9f, 21f)
                }
                PlayerSymbol.FULLSCREEN -> {
                    moveTo(9f, 4f); lineTo(4f, 4f); lineTo(4f, 9f); moveTo(15f, 4f); lineTo(20f, 4f); lineTo(20f, 9f)
                    moveTo(4f, 15f); lineTo(4f, 20f); lineTo(9f, 20f); moveTo(15f, 20f); lineTo(20f, 20f); lineTo(20f, 15f)
                }
                PlayerSymbol.EXIT_FULLSCREEN -> {
                    moveTo(4f, 9f); lineTo(9f, 9f); lineTo(9f, 4f); moveTo(15f, 4f); lineTo(15f, 9f); lineTo(20f, 9f)
                    moveTo(9f, 20f); lineTo(9f, 15f); lineTo(4f, 15f); moveTo(20f, 15f); lineTo(15f, 15f); lineTo(15f, 20f)
                }
                PlayerSymbol.STOP -> { moveTo(6f, 6f); lineTo(18f, 6f); lineTo(18f, 18f); lineTo(6f, 18f); close() }
                PlayerSymbol.CHECK -> { moveTo(5f, 12f); lineTo(10f, 17f); lineTo(19f, 7f) }
            }
        }
    }.build()
}
