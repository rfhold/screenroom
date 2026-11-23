@file:OptIn(androidx.compose.material3.ExperimentalMaterial3Api::class)

package org.screenroom.ui

import android.app.Activity
import android.content.Context
import android.content.ContextWrapper
import androidx.compose.animation.AnimatedVisibility
import androidx.compose.animation.fadeIn
import androidx.compose.animation.fadeOut
import androidx.compose.animation.core.tween
import androidx.compose.foundation.*
import androidx.compose.foundation.layout.*
import androidx.compose.foundation.shape.RoundedCornerShape
import androidx.compose.foundation.text.BasicText
import androidx.compose.material3.*
import androidx.compose.runtime.*
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.draw.clip
import androidx.compose.ui.focus.*
import androidx.compose.ui.graphics.Brush
import androidx.compose.ui.graphics.Color
import androidx.compose.ui.input.InputMode
import androidx.compose.ui.input.key.*
import androidx.compose.ui.input.pointer.*
import androidx.compose.ui.layout.onSizeChanged
import androidx.compose.ui.platform.*
import androidx.compose.ui.semantics.*
import androidx.compose.ui.text.TextStyle
import androidx.compose.ui.text.style.TextOverflow
import androidx.compose.ui.unit.*
import androidx.compose.ui.viewinterop.AndroidView
import androidx.compose.ui.window.PopupProperties
import androidx.core.view.*
import androidx.lifecycle.Lifecycle
import androidx.lifecycle.LifecycleEventObserver
import androidx.lifecycle.compose.LocalLifecycleOwner
import androidx.media3.common.util.UnstableApi
import androidx.media3.ui.AspectRatioFrameLayout
import androidx.media3.ui.PlayerView
import org.screenroom.core.PlaybackTrackKind

private val LocalPlayerControlSize = staticCompositionLocalOf<Dp?> { null }

@androidx.annotation.OptIn(UnstableApi::class)
@Composable
internal fun PlaybackScreen(playback: NativePlayback, title: String, television: Boolean, presentation: PlaybackPresentation, exit: () -> Unit) {
    val lifecycle = LocalLifecycleOwner.current.lifecycle
    DisposableEffect(playback, lifecycle) {
        val observer = LifecycleEventObserver { _, event ->
            if (event == Lifecycle.Event.ON_STOP) playback.stop("background")
        }
        lifecycle.addObserver(observer)
        onDispose { lifecycle.removeObserver(observer); playback.stop("disposed") }
    }
    val chrome = remember(playback) { PlayerChrome() }
    var toolbarHeight by remember { mutableIntStateOf(0) }
    val activity = LocalContext.current.activity()
    DisposableEffect(activity, presentation.fullscreen) {
        val window = activity?.window
        val controller = window?.let { WindowCompat.getInsetsController(it, it.decorView) }
        val bars = WindowInsetsCompat.Type.systemBars()
        val priorBehavior = controller?.systemBarsBehavior
        val wasFullscreen = window?.decorView?.let { ViewCompat.getRootWindowInsets(it)?.isVisible(bars)?.not() } ?: false
        if (presentation.fullscreen) {
            controller?.systemBarsBehavior = WindowInsetsControllerCompat.BEHAVIOR_SHOW_TRANSIENT_BARS_BY_SWIPE
            controller?.hide(bars)
        }
        onDispose {
            if (priorBehavior != null) controller.systemBarsBehavior = priorBehavior
            if (wasFullscreen) controller?.hide(bars) else controller?.show(bars)
        }
    }
    val playFocus = remember { FocusRequester() }
    val backFocus = remember { FocusRequester() }
    val stageFocus = remember { FocusRequester() }
    val keyboard = LocalInputModeManager.current.inputMode == InputMode.Keyboard
    val blocked = playback.paused || playback.busy || playback.buffering || playback.failedMedia || !playback.canControl
    LaunchedEffect(chrome.interaction, playback.playing, blocked, chrome.menu, chrome.scrubbing, chrome.keyboardFocused, chrome.keyboardEngaged) {
        chrome.idle(playback.playing, blocked)
    }
    LaunchedEffect(chrome.visible, television, playback.canControl, chrome.menu) {
        if (television && chrome.menu == null) {
            if (chrome.visible) {
                if (playback.canControl) playFocus.requestFocus() else backFocus.requestFocus()
            } else stageFocus.requestFocus()
        }
    }
    fun back() {
        chrome.reveal()
        if (!chrome.dismiss() && !presentation.exitFullscreen()) { playback.stop("exit"); exit() }
    }
    Box(Modifier.fillMaxSize().background(Color.Black)
        .pointerInput(chrome) {
            awaitPointerEventScope {
                while (true) {
                    val event = awaitPointerEvent(PointerEventPass.Initial)
                    if (event.type == PointerEventType.Move || event.type == PointerEventType.Press) {
                        chrome.pointer()
                    }
                }
            }
        }
        .onPreviewKeyEvent { event ->
            if (event.type != KeyEventType.KeyDown) false
            else {
                val hidden = chrome.key(event.key in listOf(Key.DirectionLeft, Key.DirectionRight, Key.DirectionUp, Key.DirectionDown, Key.Tab))
                when (event.key) {
                    Key.Back, Key.Escape -> { back(); true }
                    Key.MediaPlayPause, Key.Spacebar -> { playback.togglePlay(); true }
                    Key.MediaPlay -> { playback.changePlaying(true); true }
                    Key.MediaPause -> { playback.changePlaying(false); true }
                    Key.MediaStop -> { playback.stop("remote"); true }
                    Key.MediaRewind -> { playback.seekBy(-30_000); true }
                    Key.MediaFastForward -> { playback.seekBy(30_000); true }
                    else -> if (hidden) { true } else false
                }
            }
        }.focusRequester(stageFocus).focusable(!chrome.visible).semantics {
            if (!chrome.visible) {
                contentDescription = "Show playback controls"
                onClick("Show playback controls") { chrome.reveal(); true }
            }
        }) {
        // This slot and its allocation never depend on chrome, menus or fullscreen.
        if (playback.active) AndroidView(
            factory = { context -> PlayerView(context).apply {
                useController = false
                isFocusable = false
                isFocusableInTouchMode = false
                resizeMode = AspectRatioFrameLayout.RESIZE_MODE_FIT
                player = playback.player
                subtitleView?.setBottomPaddingFraction(.04f)
                keepScreenOn = true
            } },
            modifier = Modifier.fillMaxSize(),
            update = {
                it.player = playback.player
                it.keepScreenOn = !playback.paused && playback.active
                it.subtitleView?.apply { setPadding(paddingLeft, paddingTop, paddingRight, if (chrome.visible) toolbarHeight else 0) }
            },
            onRelease = { it.player = null; it.keepScreenOn = false })
        if (playback.busy || playback.buffering) Column(Modifier.align(Alignment.Center).background(Color.Black.copy(alpha = .75f), RoundedCornerShape(12.dp)).padding(20.dp),
            horizontalAlignment = Alignment.CenterHorizontally, verticalArrangement = Arrangement.spacedBy(12.dp)) {
            CircularProgressIndicator(Modifier.size(28.dp), color = Color.White, strokeWidth = 2.dp)
            Label(if (playback.busy) "Preparing playback…" else "Loading media…")
        }
        if (playerMessageOnStage(playback.message, playback.canControl) && !playback.busy) BasicText(playback.message,
            Modifier.align(Alignment.Center).widthIn(max = 440.dp).padding(24.dp).background(Color.Black.copy(alpha = .8f), RoundedCornerShape(8.dp)).padding(12.dp)
                .semantics { liveRegion = LiveRegionMode.Polite }, style = TextStyle(color = Color.White, fontSize = 15.sp), maxLines = 4, overflow = TextOverflow.Ellipsis)
        AnimatedVisibility(chrome.visible, Modifier.align(Alignment.TopCenter).fillMaxWidth(),
            enter = fadeIn(tween(150)), exit = fadeOut(tween(180))) {
            Row(Modifier.fillMaxWidth().background(Brush.verticalGradient(listOf(Color.Black.copy(alpha = .85f), Color.Transparent)))
                .windowInsetsPadding(WindowInsets.safeDrawing.only(WindowInsetsSides.Top + WindowInsetsSides.Horizontal))
                .padding(horizontal = if (television) 24.dp else 8.dp, vertical = 8.dp)
                .chromeFocus(chrome, "header", keyboard || television), verticalAlignment = Alignment.CenterVertically) {
                PlayerButton(PlayerSymbol.BACK, if (presentation.fullscreen) "Exit fullscreen" else "Back to details", television,
                    Modifier.focusRequester(backFocus), enabled = chrome.visible, click = ::back)
                BasicText(title, Modifier.weight(1f).padding(horizontal = 12.dp), style = TextStyle(color = Color.White, fontSize = if (television) 21.sp else 17.sp),
                    maxLines = 1, overflow = TextOverflow.Ellipsis)
            }
        }
        AnimatedVisibility(chrome.visible, Modifier.align(Alignment.BottomCenter).fillMaxWidth(),
            enter = fadeIn(tween(150)), exit = fadeOut(tween(180))) {
            Column(Modifier.fillMaxWidth().background(Brush.verticalGradient(listOf(Color.Transparent, Color.Black.copy(alpha = .92f))))
                .onSizeChanged { toolbarHeight = it.height }
                .windowInsetsPadding(WindowInsets.safeDrawing.only(WindowInsetsSides.Bottom + WindowInsetsSides.Horizontal))
                .padding(horizontal = if (television && LocalConfiguration.current.screenWidthDp >= 400) 28.dp else 12.dp, vertical = 12.dp)
                .chromeFocus(chrome, "toolbar", keyboard || television)) {
                val duration = playback.session?.transport?.durationMs?.let(::playbackMilliseconds)
                PlaybackSeekBar(playback, duration, television, chrome)
                Row(Modifier.fillMaxWidth(), horizontalArrangement = Arrangement.SpaceBetween) {
                    PlayerText(playbackTime(playback.position), 12)
                    PlayerText(duration?.let(::playbackTime) ?: "Unknown duration", 12)
                }
                BoxWithConstraints(Modifier.fillMaxWidth()) {
                val layout = playerToolbarLayout(maxWidth.value, television)
                CompositionLocalProvider(LocalPlayerControlSize provides layout.targetDp.dp) {
                Row(Modifier.fillMaxWidth().focusGroup(), horizontalArrangement = Arrangement.SpaceBetween, verticalAlignment = Alignment.CenterVertically) {
                    PlayerButton(if (playback.paused) PlayerSymbol.PLAY else PlayerSymbol.PAUSE, if (playback.paused) "Play" else "Pause", television,
                        Modifier.focusRequester(playFocus), enabled = playback.canControl && chrome.visible) { chrome.reveal(); playback.togglePlay() }
                    PlayerButton(PlayerSymbol.REWIND, "Back 30 seconds", television, enabled = playback.canControl && chrome.visible) { chrome.reveal(); playback.seekBy(-30_000) }
                    PlayerButton(PlayerSymbol.FORWARD, "Forward 30 seconds", television, enabled = playback.canControl && chrome.visible) { chrome.reveal(); playback.seekBy(30_000) }
                    PlayerMenuAnchor(PlayerMenu.VOLUME, if (playback.volume == 0f) PlayerSymbol.MUTED else PlayerSymbol.VOLUME,
                        if (playback.volume == 0f) "Volume, muted" else "Volume, ${(playback.volume * 100).toInt()} percent", playback, chrome, television)
                    if (layout.separateTracks) PlayerMenuAnchor(PlayerMenu.TRACKS, PlayerSymbol.TRACKS, "Audio and subtitles", playback, chrome, television)
                    PlayerMenuAnchor(PlayerMenu.SETTINGS, PlayerSymbol.SETTINGS,
                        if (layout.separateTracks) "Playback settings and recovery" else "Audio, subtitles, settings and recovery",
                        playback, chrome, television, compact = !layout.separateTracks)
                    PlayerButton(if (presentation.fullscreen) PlayerSymbol.EXIT_FULLSCREEN else PlayerSymbol.FULLSCREEN,
                        if (presentation.fullscreen) "Exit fullscreen" else "Enter fullscreen", television, enabled = chrome.visible, selected = presentation.fullscreen) {
                        chrome.reveal(); presentation.fullscreen = !presentation.fullscreen
                    }
                } } }
            }
        }
    }
}

private fun Modifier.chromeFocus(chrome: PlayerChrome, owner: String, keyboard: Boolean): Modifier =
    onFocusChanged { chrome.focus(owner, keyboard && it.hasFocus) }.focusGroup()

@Composable
private fun PlayerText(text: String, size: Int = 14) {
    BasicText(text, style = TextStyle(color = Color.White, fontSize = size.sp, lineHeight = (size + 5).sp))
}

@Composable
private fun PlayerButton(symbol: PlayerSymbol, label: String, television: Boolean, modifier: Modifier = Modifier,
    enabled: Boolean = true, selected: Boolean = false, click: () -> Unit) {
    var focused by remember { mutableStateOf(false) }
    val keyboard = LocalInputModeManager.current.inputMode == InputMode.Keyboard
    TooltipBox(positionProvider = TooltipDefaults.rememberPlainTooltipPositionProvider(),
        tooltip = { PlainTooltip { PlayerText(label, 12) } }, state = rememberTooltipState()) {
        Box(modifier.size(LocalPlayerControlSize.current ?: if (television) 56.dp else 48.dp).onFocusChanged { focused = it.isFocused }
            .clip(RoundedCornerShape(8.dp)).background(if (selected || focused && (keyboard || television)) Color.White.copy(alpha = .18f) else Color.Transparent)
            .border(if (focused && (keyboard || television)) 2.dp else 0.dp, Color.White, RoundedCornerShape(8.dp))
            .semantics { contentDescription = label; this.selected = selected }
            .clickable(enabled = enabled, role = Role.Button, onClick = click), contentAlignment = Alignment.Center) {
            Image(playerSymbols.getValue(symbol), null, Modifier.size(if (television) 28.dp else 24.dp),
                alpha = if (enabled) 1f else .4f)
        }
    }
}

@androidx.annotation.OptIn(UnstableApi::class)
@Composable
private fun PlayerMenuAnchor(menu: PlayerMenu, symbol: PlayerSymbol, label: String, playback: NativePlayback, chrome: PlayerChrome, television: Boolean, compact: Boolean = false) {
    val configuration = LocalConfiguration.current
    val menuHeight = (configuration.screenHeightDp * .58f).dp.coerceAtMost(420.dp)
    val menuWidth = (configuration.screenWidthDp - 32).dp.coerceAtMost(360.dp)
    Box {
        PlayerButton(symbol, label, television, enabled = chrome.visible, selected = chrome.menu == menu) {
            if (chrome.menu == menu) chrome.dismiss() else chrome.open(menu)
        }
        DropdownMenu(expanded = chrome.menu == menu, onDismissRequest = { chrome.dismiss() },
            modifier = Modifier.width(menuWidth).heightIn(max = menuHeight).background(Color(0xff202020)),
            properties = PopupProperties(focusable = true, dismissOnBackPress = true, dismissOnClickOutside = true)) {
            if (playerMenuIncludesTracks(menu, compact)) TrackMenuChoices(playback)
            when (menu) {
                PlayerMenu.VOLUME -> {
                    MenuChoice(if (playback.volume == 0f) "Unmute" else "Mute", enabled = playback.canControl, selected = playback.volume == 0f) {
                        playback.changeVolume(if (playback.volume == 0f) 1f else 0f)
                    }
                    PlayerText("Volume ${(playback.volume * 100).toInt()}%")
                    Slider(playback.volume, { playback.changeVolume(it); chrome.reveal() }, enabled = playback.canControl,
                        modifier = Modifier.padding(horizontal = 12.dp).semantics { contentDescription = "Volume" }, colors = monochromeSlider())
                }
                PlayerMenu.TRACKS -> Unit
                PlayerMenu.SETTINGS -> {
                    if (playback.canRetryCreate) MenuChoice("Retry preparation") { playback.retryCreate() }
                    MenuChoice("Check session state", enabled = (playback.active && !playback.busy && playback.session != null) || playback.canRecoverStop) { playback.checkSession() }
                    if (playback.canRetryTransport) MenuChoice("Retry exact transport request") { playback.retryTransport() }
                    if (playback.canFallback) MenuChoice("Try compatible playback") { playback.fallback() }
                    if (playback.cleanupOpen) {
                        MenuChoice("Retry exact stop", enabled = playback.canRecoverStop) { playback.retryStop() }
                        MenuChoice("Check cleanup state", enabled = playback.canRecoverStop) { playback.checkSession() }
                    }
                    MenuChoice("Stop playback", enabled = playback.active) { playback.stop("controls") }
                    playback.session?.let {
                        DropdownMenuItem(text = { PlayerText("State: ${it.state.name.lowercase()} · ${it.transport?.method?.name?.lowercase()?.replace('_', ' ') ?: "No transport"}\nPersonal report: ${it.reportOutcome.name.lowercase()}${if (it.cleanupPending) " · Cleanup pending" else ""}", 12) }, onClick = {}, enabled = false)
                    }
                    if (playback.message.isNotEmpty()) DropdownMenuItem(text = { PlayerText(playback.message, 12) }, onClick = {}, enabled = false)
                }
            }
            MenuChoice("Close menu") { chrome.dismiss() }
        }
    }
}

@androidx.annotation.OptIn(UnstableApi::class)
@Composable
private fun TrackMenuChoices(playback: NativePlayback) {
    for (kind in listOf(PlaybackTrackKind.AUDIO, PlaybackTrackKind.SUBTITLE)) {
        DropdownMenuItem(text = { PlayerText(if (kind == PlaybackTrackKind.AUDIO) "Audio" else "Subtitles") }, onClick = {}, enabled = false)
        val tracks = playback.session?.tracks?.filter { it.kind == kind }.orEmpty()
        if (kind == PlaybackTrackKind.SUBTITLE) MenuChoice("Off", enabled = playback.canControl, selected = tracks.none { it.selected }) { playback.choose(null) }
        if (kind == PlaybackTrackKind.AUDIO && tracks.isEmpty()) MenuChoice("No selectable audio tracks", enabled = false) {}
        tracks.forEach { track ->
            MenuChoice(listOfNotNull(track.displayName, track.language, track.subtitleMode?.name?.lowercase()?.replace('_', ' '),
                "Unavailable".takeUnless { track.available }).distinct().joinToString(" · "),
                enabled = playback.canControl && track.available, selected = track.selected) { playback.choose(track) }
        }
    }
}

@Composable
private fun MenuChoice(label: String, enabled: Boolean = true, selected: Boolean = false, click: () -> Unit) {
    var focused by remember { mutableStateOf(false) }
    val keyboard = LocalInputModeManager.current.inputMode == InputMode.Keyboard
    DropdownMenuItem(text = { PlayerText(label) }, onClick = click, enabled = enabled,
        modifier = Modifier.heightIn(min = 56.dp).onFocusChanged { focused = it.isFocused }
            .border(if (focused && keyboard) 2.dp else 0.dp, Color.White, RoundedCornerShape(6.dp))
            .semantics { this.selected = selected },
        trailingIcon = { if (selected) Image(playerSymbols.getValue(PlayerSymbol.CHECK), null, Modifier.size(20.dp)) })
}

private tailrec fun Context.activity(): Activity? = when (this) {
    is Activity -> this
    is ContextWrapper -> baseContext.activity()
    else -> null
}

@androidx.annotation.OptIn(UnstableApi::class)
@Composable
internal fun StopRecoveryActions(playback: NativePlayback, allowed: () -> Boolean = { true }) {
    if (playback.cleanupOpen) Row(Modifier.horizontalScroll(rememberScrollState()), horizontalArrangement = Arrangement.spacedBy(8.dp)) {
        Action("Retry exact stop", enabled = playback.canRecoverStop) { if (allowed()) playback.retryStop() }
        Action("Check cleanup state", enabled = playback.canRecoverStop) { if (allowed()) playback.checkSession() }
    }
}

@Composable
private fun monochromeSlider() = SliderDefaults.colors(thumbColor = Color.White, activeTrackColor = Color.White,
    inactiveTrackColor = Color.White.copy(alpha = .3f))

@androidx.annotation.OptIn(UnstableApi::class)
@Composable
private fun PlaybackSeekBar(playback: NativePlayback, duration: Long?, television: Boolean, chrome: PlayerChrome) {
    var preview by remember(playback) { mutableStateOf<Float?>(null) }
    var focused by remember { mutableStateOf(false) }
    val keyboard = LocalInputModeManager.current.inputMode == InputMode.Keyboard
    Slider(
        value = preview ?: if (duration != null && duration > 0) (playback.position.toDouble() / duration).toFloat().coerceIn(0f, 1f) else 0f,
        onValueChange = { preview = it; chrome.scrubbing = true; chrome.reveal() },
        onValueChangeFinished = {
            if (duration != null) preview?.let { playback.seekTo((it.toDouble() * duration).toLong()) }
            preview = null; chrome.scrubbing = false; chrome.reveal()
        },
        enabled = playback.canControl && duration != null && duration > 0 && chrome.visible,
        colors = monochromeSlider(),
        modifier = Modifier.fillMaxWidth().height(44.dp).semantics { contentDescription = "Playback position" }
            .onFocusChanged { focused = it.isFocused }.border(if (focused && (keyboard || television)) 2.dp else 0.dp, Color.White, RoundedCornerShape(4.dp))
            .onPreviewKeyEvent {
                if (!television || it.type != KeyEventType.KeyDown) false
                else when (it.key) {
                    Key.DirectionLeft -> { chrome.reveal(); playback.seekBy(-30_000); true }
                    Key.DirectionRight -> { chrome.reveal(); playback.seekBy(30_000); true }
                    else -> false
                }
            })
}
