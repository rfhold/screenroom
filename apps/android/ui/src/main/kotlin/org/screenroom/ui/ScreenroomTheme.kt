package org.screenroom.ui

import androidx.compose.runtime.Composable
import androidx.compose.runtime.CompositionLocalProvider
import androidx.compose.runtime.staticCompositionLocalOf
import androidx.compose.ui.graphics.Color
import androidx.compose.ui.text.TextStyle
import androidx.compose.ui.text.font.FontFamily
import androidx.compose.ui.unit.sp

internal data class ScreenroomColors(
    val background: Color = Color(0xFF101010),
    val surface: Color = Color(0xFF171717),
    val text: Color = Color(0xFFF2F2F2),
    val secondary: Color = Color(0xFFA0A0A0),
    val accent: Color = Color(0xFFA8C9FF),
)

internal val LocalScreenroomColors = staticCompositionLocalOf { ScreenroomColors() }
internal val BodyStyle = TextStyle(fontFamily = FontFamily.SansSerif, fontSize = 16.sp, lineHeight = 24.sp)

@Composable
fun ScreenroomTheme(content: @Composable () -> Unit) {
    CompositionLocalProvider(LocalScreenroomColors provides ScreenroomColors(), content = content)
}
