package org.screenroom.e2e

import android.app.Activity
import android.graphics.Bitmap
import android.view.KeyEvent
import androidx.compose.ui.semantics.SemanticsActions
import androidx.compose.ui.semantics.SemanticsProperties
import androidx.compose.ui.semantics.getOrNull
import androidx.compose.ui.test.*
import androidx.compose.ui.test.junit4.ComposeTestRule
import androidx.test.platform.app.InstrumentationRegistry
import java.io.File
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.runBlocking
import org.json.JSONArray
import org.json.JSONObject
import org.screenroom.core.*

fun ComposeTestRule.assertShowcaseSearch(activity: Activity) = assertPlaybackShowcase(activity, searchOnly = true)

fun ComposeTestRule.assertShowcasePlayback(activity: Activity) = assertPlaybackShowcase(activity, playbackOnly = true)

fun ComposeTestRule.assertPlaybackShowcase(activity: Activity, searchOnly: Boolean = false, playbackOnly: Boolean = false) {
    require(!(searchOnly && playbackOnly))
    val instrumentation = InstrumentationRegistry.getInstrumentation()
    fun arg(name: String) = requireNotNull(InstrumentationRegistry.getArguments().getString(name)).also { require(it.isNotBlank()) }
    val id = arg("SCREENROOM_SHOWCASE_ITEM_ID")
    val title = arg("SCREENROOM_SHOWCASE_TITLE")
    val year = arg("SCREENROOM_SHOWCASE_YEAR").toInt()
    val slug = if (activity.packageManager.hasSystemFeature("android.software.leanback")) "android-tv" else "android"
    val run = arg("SCREENROOM_SHOWCASE_RUN")
    require(run.matches(Regex("[a-zA-Z0-9_-]+")))
    val directory = File(activity.filesDir, if (searchOnly || playbackOnly) "showcase-search/$run" else "screenshots/playback/$run")
    check(directory.mkdirs())
    val states = JSONArray()
    val recorded = mutableSetOf<String>()
    var viewport = "unknown"
    fun exists(matcher: SemanticsMatcher) = onAllNodes(matcher).fetchSemanticsNodes().isNotEmpty()
    fun text(value: String) = exists(hasText(value, substring = true))
    fun visibleTexts() = onAllNodes(isRoot(), useUnmergedTree = true).fetchSemanticsNodes()
        .flatMap { root -> fun walk(node: androidx.compose.ui.semantics.SemanticsNode): List<String> =
            node.config.getOrNull(SemanticsProperties.Text).orEmpty().map { it.text } + node.children.flatMap { walk(it) }
            walk(root) }.distinct()
    fun row(key: String, status: String, observation: String, reason: String? = null, label: String = key.replace('-', ' ')) {
        check(recorded.add(key))
        val state = JSONObject().put("id", key).put("label", label).put("status", status)
            .put("viewport", viewport).put("observation", observation)
        if (reason != null) state.put("reason", reason)
        if (status == "captured") {
            check(instrumentation.uiAutomation.rootInActiveWindow?.packageName?.toString() == activity.packageName) { "Application does not own foreground window" }
            val image = requireNotNull(instrumentation.uiAutomation.takeScreenshot())
            val location = IntArray(2)
            var width = 0
            var height = 0
            runOnIdle { activity.window.decorView.getLocationOnScreen(location); width = activity.window.decorView.width; height = activity.window.decorView.height }
            val app = Bitmap.createBitmap(image, location[0], location[1], width, height)
            viewport = "${app.width}x${app.height}"
            state.put("viewport", viewport).put("image", "$slug/$key.png")
            File(directory, "$key.png").outputStream().use { check(app.compress(Bitmap.CompressFormat.PNG, 100, it)) }
            if (app !== image) app.recycle()
            image.recycle()
        }
        states.put(state)
    }
    fun click(matcher: SemanticsMatcher) {
        val node = onNode(matcher)
        try { node.assertIsDisplayed() } catch (_: AssertionError) { node.performScrollTo().assertIsDisplayed() }
        node.assertIsEnabled().performClick()
        waitForIdle()
    }
    fun closeMenu() { if (text("Close menu")) click(hasText("Close menu")) }
    fun reveal() { if (exists(hasContentDescription("Show playback controls"))) click(hasContentDescription("Show playback controls")) }
    fun settings() {
        reveal()
        click(hasContentDescription("Playback settings and recovery") or hasContentDescription("Audio, subtitles, settings and recovery"))
    }
    fun errorVisible() = text("Media could not be played.")
    fun selectionEvidence(node: SemanticsNodeInteraction) = trackSelectionEvidence(
        selected = node.fetchSemanticsNode().config.getOrNull(SemanticsProperties.Selected) == true,
        controlsEnabled = node.isEnabled() && onNodeWithContentDescription("Playback position").isEnabled(),
        preparing = text("Preparing playback…"),
        loading = text("Loading media…"),
        mediaError = errorVisible(),
        transportUnconfirmed = text("Transport change is unconfirmed."),
    )
    fun searchCard(item: Item) = ShowcaseSearchCard(item.id, item.title, item.year.toString(),
        showcaseCardDescription(item.title, item.kind, item.subtitle, item.watched == true, item.progressPercent?.toInt()))
    val required = listOf("playing", "paused", "idle", "seek-before", "seek-after", "volume-0", "volume-50", "volume-100", "mute", "tracks", "subtitles-off", "settings", "fullscreen", "loading", "error", "stop", "cleanup")
    var started = false
    var blocked = false
    var focusedCleanup = "not-started"
    fun stopFocusedPlayback() {
        started = false
        focusedCleanup = "attempted-provider-unconfirmed"
        try {
            settings()
            val stop = onNodeWithText("Stop playback")
            if (stop.isEnabled()) {
                stop.performScrollTo().performClick()
                waitUntil(15_000) { visibleTexts().any { it.contains("stopped", true) || it.contains("cleanup", true) } }
                focusedCleanup = "stop-observed-provider-unconfirmed"
            } else focusedCleanup = "stop-disabled-provider-unconfirmed"
        } finally {
            closeMenu()
            reveal()
            click(hasContentDescription("Back to details"))
            waitUntil(15_000) { exists(hasTestTag("detail-header")) }
        }
    }
    var searchStage = "catalog-preflight"
    val diagnostics = ShowcaseSearchDiagnostics(this, instrumentation, File(activity.filesDir, "showcase-search/$run"), title)
    try {
        val (item, selectedCard) = runBlocking(Dispatchers.IO) {
            val client = Client(ClientConfig(arg("SCREENROOM_API_ENDPOINT"), arg("SCREENROOM_USER_ID"), 5000u))
            try {
                val capabilities = client.getSearchCapabilities()
                diagnostics.capabilities(capabilities.methods, capabilities.types, capabilities.sources)
                check(capabilities.methods.containsAll(listOf("title_only", "text", "fuzzy")) &&
                    capabilities.types.map { it.lowercase() }.containsAll(listOf("movie", "series", "episode")) &&
                    "all" in capabilities.sources) { "Required showcase search capabilities are unavailable" }
                val selected = client.getItem(id, SourceLibraryScope("", ""))
                check(selected.id == id && selected.title == title && selected.year.toString() == year.toString()) { "Explicit showcase selector differs from catalog item" }
                val results = mutableListOf<ShowcaseSearchCard>()
                val tokens = mutableSetOf<String>()
                var token = ""
                var pages = 0
                do {
                    check(pages++ < 16 && tokens.add(token)) { "Title search exceeded sixteen pages or repeated a continuation token" }
                    val result = client.search(SearchRequest(title, "", SearchTypes(movie = true, series = true, episode = false),
                        SearchMethods(titleOnly = true, text = true, fuzzy = true, semantic = false), "all", Page(100u, token), ""))
                    results += result.items.map(::searchCard)
                    token = result.nextPageToken
                } while (token.isNotEmpty())
                selected to selectShowcaseSearchCard(id, title, year.toString(), searchCard(selected), results)
            } finally { client.cancelAll(); client.close() }
        }
        check(item.id == id && item.title == title && item.year.toString() == year.toString()) { "Explicit showcase selector differs from catalog item" }
        waitUntil(15_000) { exists(hasTestTag("home-scope")) }
        val buttonRole = SemanticsMatcher.expectValue(SemanticsProperties.Role, androidx.compose.ui.semantics.Role.Button)
        val checkboxRole = SemanticsMatcher.expectValue(SemanticsProperties.Role, androidx.compose.ui.semantics.Role.Checkbox)
        val searchContent = hasAnyAncestor(hasTestTag("content"))
        fun navigationButton(label: String, scope: String) =
            hasContentDescription(label) and hasClickAction() and buttonRole and hasAnyAncestor(hasTestTag(scope))
        fun clickNavigation(label: String, scope: String) {
            val matcher = navigationButton(label, scope)
            onAllNodes(matcher).assertCountEquals(1)
            click(matcher)
        }
        fun setSearchOption(label: String, enabled: Boolean) {
            searchStage = "option-$label"
            val matcher = hasContentDescription(label) and isToggleable() and checkboxRole and searchContent
            diagnostics.snapshot("$searchStage-before")
            onAllNodes(matcher).assertCountEquals(1)
            val choice = onNode(matcher)
            if ((choice.fetchSemanticsNode().config[SemanticsProperties.ToggleableState] == androidx.compose.ui.state.ToggleableState.On) != enabled) {
                choice.performScrollTo().assertIsDisplayed()
                choice.assertIsEnabled().performClick()
                waitForIdle()
            }
            waitUntil(5_000) {
                val nodes = onAllNodes(matcher).fetchSemanticsNodes()
                showcaseSearchOptionSettled(nodes.map {
                    when (it.config.getOrNull(SemanticsProperties.ToggleableState)) {
                        androidx.compose.ui.state.ToggleableState.On -> true
                        androidx.compose.ui.state.ToggleableState.Off -> false
                        else -> null
                    }
                }, enabled)
            }
            choice.assertIsOnOrOff(enabled)
            diagnostics.snapshot("$searchStage-settled")
        }
        searchStage = "search-navigation"
        if (exists(navigationButton("Expand sidebar", "sidebar"))) clickNavigation("Expand sidebar", "sidebar")
        clickNavigation("Search titles", "sidebar")
        clickNavigation("Search settings", "content")
        setSearchOption("Title only", true)
        // The Text label depends on the actual settled Title only state.
        onNode(hasContentDescription("Title only") and isToggleable() and checkboxRole and searchContent).assertIsOn()
        for ((label, enabled) in listOf("Movies" to true, "Shows" to true, "Episodes" to false,
            "Text (titles only)" to true, "Fuzzy titles" to true)) {
            setSearchOption(label, enabled)
        }
        setSearchOption("All", true)
        clickNavigation("Search settings", "content")
        searchStage = "exact-card-query"
        onAllNodesWithTag("search-query").assertCountEquals(1)
        onNodeWithTag("search-query").performTextReplacement(title)
        val cardMatcher = hasTestTag(selectedCard.selector) and hasClickAction() and hasAnyAncestor(hasTestTag("content"))
        var uiPages = 0
        while (true) {
            check(uiPages++ < 16) { "Explicit UI card not found within sixteen search pages" }
            waitUntil(15_000) { exists(hasTestTag("search-effective-methods")) && !text("Searching titles…") && !text("Loading items…") }
            if (exists(cardMatcher)) break
            check(exists(navigationButton("Load more", "content"))) { "Explicit search result is missing from actual UI cards" }
            clickNavigation("Load more", "content")
        }
        onAllNodes(cardMatcher).assertCountEquals(1)
        val card = onNode(cardMatcher)
        card.performScrollTo().assertIsDisplayed()
        fun resourceMatches(node: android.view.accessibility.AccessibilityNodeInfo?): Int {
            if (node == null) return 0
            val own = if (node.viewIdResourceName == selectedCard.selector && node.isClickable) 1 else 0
            return own + (0 until node.childCount).sumOf { resourceMatches(node.getChild(it)) }
        }
        waitUntil(5_000) { resourceMatches(instrumentation.uiAutomation.rootInActiveWindow) == 1 }
        card.performClick()
        waitUntil(15_000) { exists(hasTestTag("detail-header")) }
        onNode(hasText("$title") and hasAnyAncestor(hasTestTag("title-details"))).assertIsDisplayed()
        onNodeWithText("${item.kind.replaceFirstChar { it.uppercase() }} · $year").assertExists()
        searchStage = "exact-details"
        diagnostics.snapshot(searchStage)
        if (searchOnly) {
            File(directory, "result.json").writeText(JSONObject().put("status", "search-observed")
                .put("item_id", id).put("title", title).put("year", year).put("play_invoked", false).toString(2))
            return
        }
        searchStage = "playback"
        runOnIdle { activity.window.decorView.windowInsetsController?.hide(android.view.WindowInsets.Type.ime()) }
        click(hasText("Play") or hasText("Resume"))
        started = true
        waitUntil(30_000) { text("Preparing playback…") || exists(hasContentDescription("Pause")) || errorVisible() }
        if (!playbackOnly && (text("Preparing playback…") || text("Loading media…"))) row("loading", "captured", "Visible playback loading indicator")
        waitUntil(45_000) { errorVisible() || (!text("Preparing playback…") && !text("Loading media…") && exists(hasContentDescription("Pause"))) }
        blocked = errorVisible()
        if (playbackOnly) {
            check(!blocked) { "Natural media error visible; no retry attempted" }
            reveal()
            onNodeWithContentDescription("Pause").assertIsEnabled()
            val before = onNodeWithContentDescription("Playback position").fetchSemanticsNode().config[SemanticsProperties.ProgressBarRangeInfo].current
            var after = before
            waitUntil(10_000) {
                blocked = errorVisible()
                check(!blocked) { "Natural media error while observing position" }
                after = onNodeWithContentDescription("Playback position").fetchSemanticsNode().config[SemanticsProperties.ProgressBarRangeInfo].current
                after > before
            }
            stopFocusedPlayback()
            check(focusedCleanup == "stop-observed-provider-unconfirmed") { "Normal Stop was not observed" }
            File(directory, "result.json").writeText(JSONObject().put("status", "playback-observed")
                .put("item_id", id).put("title", title).put("year", year).put("play_invoked", true)
                .put("pause_enabled", true).put("position_before", before).put("position_after", after)
                .put("cleanup", focusedCleanup).put("back_to_details", true).toString(2))
            return
        }
        if (blocked) row("error", "captured", "Natural media error visible; no fallback or retry attempted")
        else {
            reveal()
            onNodeWithContentDescription("Pause").assertIsEnabled()
            val before = onNodeWithContentDescription("Playback position").fetchSemanticsNode().config[SemanticsProperties.ProgressBarRangeInfo].current
            waitUntil(10_000) {
                onNodeWithContentDescription("Playback position").fetchSemanticsNode().config[SemanticsProperties.ProgressBarRangeInfo].current > before
            }
            row("playing", "captured", "Pause control enabled and displayed playback position advanced")
            val idleDeadline = android.os.SystemClock.uptimeMillis() + 8_000
            while (!exists(hasContentDescription("Show playback controls")) && android.os.SystemClock.uptimeMillis() < idleDeadline) {
                Thread.sleep(200)
            }
            if (exists(hasContentDescription("Show playback controls"))) row("idle", "captured", "Player stage exposes Show playback controls after natural idle")
            reveal()
            click(hasContentDescription("Pause"))
            onNodeWithContentDescription("Play").assertIsEnabled()
            row("paused", "captured", "Play control visible after normal Pause action")
            row("seek-before", "captured", "Paused position before Forward 30 seconds")
            val position = onNodeWithContentDescription("Playback position").fetchSemanticsNode().config[SemanticsProperties.ProgressBarRangeInfo].current
            click(hasContentDescription("Forward 30 seconds"))
            waitUntil(15_000) { onNodeWithContentDescription("Playback position").fetchSemanticsNode().config[SemanticsProperties.ProgressBarRangeInfo].current > position }
            row("seek-after", "captured", "Displayed playback position advanced after Forward 30 seconds")
            click(hasContentDescription("Volume, 100 percent"))
            for ((value, key) in listOf(0f to "volume-0", .5f to "volume-50", 1f to "volume-100")) {
                onNodeWithContentDescription("Volume").performSemanticsAction(SemanticsActions.SetProgress) { check(it(value)) }
                onNodeWithText("Volume ${(value * 100).toInt()}%").assertExists()
                row(key, "captured", "Volume UI reports ${(value * 100).toInt()} percent")
            }
            click(hasText("Mute"))
            onNodeWithText("Volume 0%").assertExists()
            onNodeWithText("Unmute").assertExists()
            row("mute", "captured", "Mute action produced Volume 0% and Unmute control")
            closeMenu()
            reveal()
            val separate = exists(hasContentDescription("Audio and subtitles"))
            if (separate) click(hasContentDescription("Audio and subtitles")) else settings()
            row("tracks", "captured", "Actual audio/subtitle menu; only displayed choices are evidenced")
            val choices = onAllNodes(hasClickAction()).fetchSemanticsNodes().mapNotNull { node ->
                if (!showcaseTrackChoiceSemantics(
                        node.config.getOrNull(SemanticsProperties.Selected),
                        node.config.contains(SemanticsProperties.Heading)
                    )) return@mapNotNull null
                val name = node.config.getOrNull(SemanticsProperties.Text)?.joinToString(" ") { it.text } ?: return@mapNotNull null
                if (name in listOf("Close menu", "Off", "Check session state", "Stop playback") || name.startsWith("State:")) null
                else name to (node.config.getOrNull(SemanticsProperties.Disabled) == null)
            }
            fun captureChoice(key: String, name: String) {
                if (blocked) {
                    row(key, "blocked", "Available choice was not selected after media failure", "Natural media error prevented this state", name)
                    return
                }
                val node = onNodeWithText(name)
                node.performScrollTo().assertIsEnabled()
                val alreadySelected = node.fetchSemanticsNode().config.getOrNull(SemanticsProperties.Selected) == true
                if (!alreadySelected) node.performClick()
                var evidence = TrackSelectionEvidence.WAIT
                waitUntil(30_000) {
                    evidence = selectionEvidence(node)
                    evidence != TrackSelectionEvidence.WAIT
                }
                when (evidence) {
                    TrackSelectionEvidence.SETTLED -> row(key, "captured",
                        if (alreadySelected) "Current selected configuration; controls enabled and media loading settled; no update issued"
                        else "Requested choice selected; controls enabled and media loading settled after normal menu update", label = name)
                    TrackSelectionEvidence.MEDIA_BLOCKED -> {
                        blocked = true
                        closeMenu()
                        onNodeWithText("Media could not be played.", substring = true).assertIsDisplayed()
                        if ("error" !in recorded) row("error", "captured", "Natural media error visible after track selection; no fallback or retry attempted")
                        row(key, "blocked", "Requested configuration did not reach settled media state", "Natural media error after track selection", name)
                    }
                    TrackSelectionEvidence.UNCONFIRMED -> error("Track transport change is unconfirmed; no retry or capture permitted")
                    TrackSelectionEvidence.WAIT -> error("Track selection did not settle")
                }
            }
            for ((index, choice) in choices.withIndex()) {
                val (name, enabled) = choice
                check("://" !in name && name.length <= 240) { "Unsafe track label" }
                if (enabled) {
                    captureChoice("track-choice-${index + 1}", name)
                } else row("track-choice-${index + 1}", "unavailable", "Track choice disabled in actual menu", "Disabled by application", name)
            }
            captureChoice("subtitles-off", "Off")
            closeMenu()
            if (!blocked) {
                reveal()
                click(hasContentDescription("Enter fullscreen"))
                onNode(hasContentDescription("Exit fullscreen") and isSelected()).assertExists()
                var hidden = false
                waitUntil(5_000) { runOnIdle { hidden = activity.window.decorView.rootWindowInsets?.isVisible(android.view.WindowInsets.Type.systemBars()) == false }; hidden }
                row("fullscreen", "captured", "Fullscreen control selected and system bars hidden")
                onAllNodesWithContentDescription("Exit fullscreen")[0].performClick()
            }
        }
        settings()
        row("settings", "captured", "Actual recovery/settings menu; no recovery action invoked")
        val stop = onNodeWithText("Stop playback")
        if (stop.isEnabled()) {
            stop.performScrollTo().performClick()
            waitUntil(15_000) { visibleTexts().any { it.contains("stopped", true) || it.contains("cleanup", true) } }
            row("stop", "captured", "Normal Stop playback invoked; provider confirmation not inferred")
        } else row("stop", "unavailable", "Stop control disabled", "Media session already locally inactive")
        closeMenu()
        reveal()
        click(hasContentDescription("Back to details"))
        waitUntil(15_000) { exists(hasTestTag("detail-header")) }
        row("cleanup", "captured", "Returned to details via owned Back; provider cleanup remains unconfirmed unless UI explicitly reports it")
        started = false
        for (key in required.filterNot { it in recorded }) row(key, if (blocked && key !in listOf("loading", "idle")) "blocked" else "not-observed",
            "No capture claimed", if (blocked) "Natural media error prevented this state" else "State was not observed through available normal UI")
        val platform = JSONObject().put("slug", slug).put("runtime", "Android ${android.os.Build.VERSION.RELEASE}; ${android.os.Build.MODEL}")
            .put("outcome", if (blocked) "blocked" else "observed").put("states", states)
        File(directory, "manifest.json").writeText(JSONObject().put("schema_version", 1).put("journey", "playback")
            .put("backend_instance_id", arg("SCREENROOM_BACKEND_INSTANCE_ID"))
            .put("item", JSONObject().put("id", id).put("title", title).put("year", year))
            .put("platforms", JSONArray().put(platform)).toString(2))
    } catch (failure: Throwable) {
        if (playbackOnly) {
            if (started) try { stopFocusedPlayback() } catch (cleanupFailure: Throwable) { failure.addSuppressed(cleanupFailure) }
            try {
                File(directory, "result.json").writeText(JSONObject().put("status", "failed")
                    .put("media_error_visible", blocked).put("cleanup", focusedCleanup)
                    .put("failure_class", when (failure) {
                        is AssertionError -> "AssertionError"
                        is IllegalStateException -> "IllegalStateException"
                        else -> "HarnessFailure"
                    }).toString(2))
            } catch (diagnosticFailure: Throwable) { failure.addSuppressed(diagnosticFailure) }
        }
        try { diagnostics.snapshot("failure-$searchStage") } catch (diagnosticFailure: Throwable) { failure.addSuppressed(diagnosticFailure) }
        throw failure
    } finally {
        if (started) {
            closeMenu()
            instrumentation.sendKeyDownUpSync(KeyEvent.KEYCODE_MEDIA_STOP)
            instrumentation.sendKeyDownUpSync(KeyEvent.KEYCODE_BACK)
        }
    }
}

private fun SemanticsNodeInteraction.isEnabled(): Boolean =
    fetchSemanticsNode().config.getOrNull(SemanticsProperties.Disabled) == null

private fun SemanticsNodeInteraction.assertIsOnOrOff(on: Boolean) {
    if (on) assertIsOn() else assertIsOff()
}
