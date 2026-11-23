package org.screenroom.e2e

import android.app.Activity
import android.content.Intent
import android.graphics.Bitmap
import android.view.KeyEvent
import androidx.activity.ComponentActivity
import androidx.activity.compose.setContent
import androidx.compose.runtime.*
import androidx.compose.ui.semantics.SemanticsActions
import androidx.compose.ui.semantics.SemanticsProperties
import androidx.compose.ui.semantics.getOrNull
import androidx.compose.ui.test.*
import androidx.compose.ui.test.junit4.ComposeTestRule
import androidx.test.platform.app.InstrumentationRegistry
import java.io.File
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.runBlocking
import org.junit.Assert.assertEquals
import org.junit.Assert.assertTrue
import org.screenroom.core.*

private fun argument(name: String): String = requireNotNull(InstrumentationRegistry.getArguments().getString(name)) { "Supply $name from the configured installation" }.also { require(it.isNotBlank()) }

fun <A : ComponentActivity> coreComposeRule(activity: Class<A>, profile: String): androidx.compose.ui.test.junit4.AndroidComposeTestRule<androidx.test.ext.junit.rules.ActivityScenarioRule<A>, A> {
    val instrumentation = InstrumentationRegistry.getInstrumentation()
    val intent = Intent(instrumentation.targetContext, activity)
        .putExtra("SCREENROOM_API_ENDPOINT", argument("SCREENROOM_API_ENDPOINT"))
        .putExtra("SCREENROOM_USER_ID", argument("SCREENROOM_USER_ID"))
        .putExtra("SCREENROOM_E2E_SETTINGS_NAMESPACE", InstrumentationRegistry.getArguments().getString("SCREENROOM_E2E_SETTINGS_NAMESPACE") ?: profile)
    return androidx.compose.ui.test.junit4.AndroidComposeTestRule(androidx.test.ext.junit.rules.ActivityScenarioRule<A>(intent)) { rule ->
        var current: A? = null
        rule.scenario.onActivity { current = it }
        current!!
    }
}

private data class RealSelection(val library: MediaLibrary, val item: Item)
private data class RealHierarchy(val library: MediaLibrary, val series: Item, val season: Item, val episode: Item)

private fun cardDescription(item: Item, seasonContext: Boolean = false): String {
    val playback = listOfNotNull(if (item.watched == true) "Watched" else null, item.progressPercent?.let { "Playback progress $it%" }).joinToString(", ")
    if (item.kind != "Episode") return listOf(item.title, item.kind, item.subtitle, playback).filter { it.isNotEmpty() }.joinToString(", ")
    val match = Regex("^(?:(.+) · )?S(\\d+)\\s*E(\\d+)(?: · .*)?$").matchEntire(item.subtitle.trim())
    val heading = if (match == null) "Episode" else {
        val episode = "E${match.groupValues[3].padStart(2, '0')}"
        if (seasonContext) episode else listOf(match.groupValues[1], "S${match.groupValues[2].padStart(2, '0')}:$episode").filter { it.isNotEmpty() }.joinToString(" · ")
    }
    return listOf(heading, item.title, playback).filter { it.isNotEmpty() }.joinToString(", ")
}

private fun <T> withClient(block: suspend (Client) -> T): T = runBlocking(Dispatchers.IO) {
    val client = Client(ClientConfig(argument("SCREENROOM_API_ENDPOINT"), argument("SCREENROOM_USER_ID"), 5000u))
    try { block(client) } finally { client.cancelAll(); client.close() }
}

private fun discover(): RealSelection = withClient { client ->
    val libraries = client.listLibraries("", Page(8u, "")).libraries
    for (library in libraries.take(8)) {
        val items = client.listLibraryItems(SourceLibraryScope("", library.id), Page(100u, "")).items
        for (item in items.filter { it.kind == "Movie" && it.artwork != null && it.title.isNotBlank() }.take(3)) {
            val results = client.search(SearchRequest(item.title, library.id, SearchTypes(true, true, false), SearchMethods(true, true, true, false), "", Page(100u, ""), ""))
            if (results.items.firstOrNull()?.id == item.id && items.count { cardDescription(it) == cardDescription(item) } == 1 && results.items.count { cardDescription(it) == cardDescription(item) } == 1) return@withClient RealSelection(library, item)
        }
    }
    error("Showcase requires an artwork-backed movie on a first library page that ranks first in scoped title search; checked at most eight libraries and three candidates each")
}

private fun discoverHierarchy(): RealHierarchy = withClient { client ->
    val libraries = client.listLibraries("", Page(8u, "")).libraries
    for (library in libraries.take(8)) {
        val scope = SourceLibraryScope("", library.id)
        val items = client.listLibraryItems(scope, Page(100u, "")).items
        for (series in items.filter { it.kind == "Series" }.take(3)) {
            val seasons = client.listChildren(series.id, Page(100u, ""), scope).items
            for (season in seasons.filter { it.kind == "Season" }.take(3)) {
                val episodes = client.listChildren(season.id, Page(100u, ""), scope).items
                val episode = episodes.firstOrNull { it.kind == "Episode" }
                if (episode != null) return@withClient RealHierarchy(library, series, season, episode)
            }
        }
    }
    error("Hierarchy regression requires an indexed real series, season and episode")
}

private fun ComposeTestRule.awaitData() {
    waitForIdle()
    waitUntil(15_000) { onAllNodes(hasText("Loading", substring = true) or hasText("Reading catalog…") or hasText("Saving preferences…")).fetchSemanticsNodes().isEmpty() }
}

private fun ComposeTestRule.activate(node: SemanticsNodeInteraction) {
    try { node.assertIsDisplayed() } catch (_: AssertionError) { node.performScrollTo().assertIsDisplayed() }
    node.performSemanticsAction(SemanticsActions.OnClick) { it() }
    awaitData()
}

private fun ComposeTestRule.awaitHome() {
    waitUntil(15_000) { onAllNodesWithTag("home-scope").fetchSemanticsNodes().isNotEmpty() }
    onNodeWithText("Home").assertIsDisplayed()
}

private fun ComposeTestRule.itemCard(item: Item, seasonContext: Boolean = false): SemanticsNodeInteraction =
    onNode(hasContentDescription(cardDescription(item, seasonContext)) and hasClickAction())

private fun ComposeTestRule.awaitSelectedCard(item: Item) {
    waitUntil(15_000) { onAllNodes(hasContentDescription(cardDescription(item)) and hasClickAction()).fetchSemanticsNodes().size == 1 }
    itemCard(item).assertExists()
}

private fun ComposeTestRule.openLibrary(selection: RealSelection) {
    if (onAllNodesWithContentDescription("Expand sidebar").fetchSemanticsNodes().isNotEmpty()) activate(onNodeWithContentDescription("Expand sidebar"))
    val occurrence = withClient { client ->
        val libraries = client.listLibraries("", Page(8u, "")).libraries
        val index = libraries.indexOfFirst { it.id == selection.library.id }
        check(index >= 0) { "Selected library left the bounded discovery page" }
        libraries.take(index).count { it.title == selection.library.title }
    }
    val buttons = onAllNodes(hasContentDescription(selection.library.title) and hasAnyAncestor(hasTestTag("sidebar")) and !isToggleable())
    waitUntil(15_000) { buttons.fetchSemanticsNodes().size > occurrence }
    activate(buttons[occurrence])
    awaitSelectedCard(selection.item)
}

private fun ComposeTestRule.openDetail(selection: RealSelection, tv: Boolean) {
    val card = itemCard(selection.item)
    if (tv) {
        card.performScrollTo().performSemanticsAction(SemanticsActions.RequestFocus) { it() }
        card.assertIsFocused().assertIsDisplayed()
        InstrumentationRegistry.getInstrumentation().sendKeyDownUpSync(KeyEvent.KEYCODE_DPAD_CENTER)
        awaitData()
    } else activate(card)
    waitUntil(15_000) { onAllNodesWithTag("detail-header").fetchSemanticsNodes().isNotEmpty() }
    onNode(hasText(selection.item.title) and hasAnyAncestor(hasTestTag("title-details"))).assertIsDisplayed()
    onNode(hasText("${selection.item.kind} · ${selection.item.year}") and hasAnyAncestor(hasTestTag("title-details"))).assertExists()
    if (selection.item.artwork != null) waitUntil(15_000) {
        onAllNodes(hasTestTag("art-ready") and hasAnyAncestor(hasTestTag("detail-poster"))).fetchSemanticsNodes().isNotEmpty()
    }
    selection.item.artwork?.let { onNode(hasContentDescription(it.label) and hasAnyAncestor(hasTestTag("detail-poster"))).assertExists() }
    onNodeWithText("Play", substring = false).assertExists()
    onNodeWithText("Resume", substring = false).assertDoesNotExist()
    val header = onNodeWithTag("detail-header").fetchSemanticsNode().boundsInRoot
    val poster = onNodeWithTag("detail-poster").fetchSemanticsNode().boundsInRoot
    val summary = onNodeWithTag("title-details").fetchSemanticsNode().boundsInRoot
    assertTrue(kotlin.math.abs(poster.height / poster.width - 1.5f) < .02f)
    assertTrue(summary.left >= poster.right || summary.top >= poster.bottom)
    assertTrue(header.width >= poster.width)
}

private fun ComposeTestRule.search(selection: RealSelection) {
    activate(onNodeWithContentDescription("Search titles"))
    activate(onNodeWithContentDescription("Search settings"))
    val titleOnly = onNodeWithContentDescription("Title only")
    if (titleOnly.fetchSemanticsNode().config[SemanticsProperties.ToggleableState] != androidx.compose.ui.state.ToggleableState.On) activate(titleOnly)
    onNodeWithContentDescription("Semantic").assertIsNotEnabled()
    activate(onNodeWithContentDescription("Search settings"))
    onNodeWithTag("search-query").performTextReplacement(selection.item.title)
    awaitData()
    awaitSelectedCard(selection.item)
    itemCard(selection.item).assertExists()
}

private fun ComposeTestRule.assertSearchReturn(selection: RealSelection, activity: Activity) {
    search(selection)
    openDetail(selection, activity.packageManager.hasSystemFeature("android.software.leanback"))
    activate(onNodeWithContentDescription("Search results"))
    onNodeWithTag("search-query").assertTextEquals(selection.item.title)
    activate(onNodeWithContentDescription("Clear query"))
    onNodeWithText("Enter a title to search ${selection.library.title}.").assertExists()
    onNode(hasContentDescription(selection.item.title, substring = true) and hasAnyAncestor(hasTestTag("content")) and hasClickAction()).assertDoesNotExist()
    activate(onNodeWithContentDescription("Search settings"))
    val movie = onNode(hasContentDescription("Movies") and isToggleable())
    val shows = onNode(hasContentDescription("Shows") and isToggleable())
    val episodes = onNode(hasContentDescription("Episodes") and isToggleable())
    activate(shows)
    activate(episodes)
    movie.assertIsOn().assertIsNotEnabled()
    activate(onNodeWithContentDescription("Fuzzy titles"))
    onNodeWithContentDescription("Text (titles only)").assertIsNotEnabled()
    activate(shows)
    activate(episodes)
    activate(onNodeWithContentDescription("Fuzzy titles"))
    activate(onNodeWithContentDescription("Search settings"))
}

private fun ComposeTestRule.journey(activity: Activity, captures: Boolean) {
    val selection = discover()
    val tv = activity.packageManager.hasSystemFeature("android.software.leanback")
    awaitHome()
    awaitData()
    val homeItems = if (captures) withClient { client -> client.getHome(SourceLibraryScope("", "")).filter { it.readError == null }.flatMap { it.items } } else emptyList()
    capture("home", captures, homeItems)
    activate(onNodeWithContentDescription("Expand sidebar"))
    onNodeWithContentDescription("Log out").assertDoesNotExist()
    if (tv) {
        val sidebar = onNodeWithTag("sidebar").fetchSemanticsNode().boundsInRoot
        val content = onNodeWithTag("content").fetchSemanticsNode().boundsInRoot
        assertTrue(sidebar.width / (sidebar.width + content.width) in .18f.. .20f)
        onNodeWithContentDescription("Home").assertIsSelected()
        onNodeWithContentDescription("Home").performSemanticsAction(SemanticsActions.RequestFocus) { it() }
        InstrumentationRegistry.getInstrumentation().sendKeyDownUpSync(KeyEvent.KEYCODE_DPAD_RIGHT)
        awaitData()
        onNodeWithContentDescription("Browse libraries").assertIsFocused()
        onNodeWithContentDescription("Expand sidebar").assertIsDisplayed()
        InstrumentationRegistry.getInstrumentation().sendKeyDownUpSync(KeyEvent.KEYCODE_DPAD_LEFT)
        awaitData()
        onNodeWithContentDescription("Collapse sidebar").assertIsFocused()
        onNodeWithContentDescription("Home").assertIsSelected()
    }
    capture("sidebar", captures, homeItems)
    activate(onNodeWithContentDescription("Collapse sidebar"))
    openLibrary(selection)
    if (onAllNodesWithContentDescription("Collapse sidebar").fetchSemanticsNodes().isNotEmpty()) activate(onNodeWithContentDescription("Collapse sidebar"))
    val libraryItems = if (captures) withClient { client -> client.listLibraryItems(SourceLibraryScope("", selection.library.id), Page(100u, "")).items } else emptyList()
    capture("library", captures, libraryItems)
    search(selection)
    runOnIdle { activity.window.decorView.windowInsetsController!!.hide(android.view.WindowInsets.Type.ime()) }
    waitUntil(15_000) { var hidden = false; runOnIdle { hidden = activity.window.decorView.rootWindowInsets?.isVisible(android.view.WindowInsets.Type.ime()) == false }; hidden }
    val searchItems = if (captures) withClient { client -> client.search(SearchRequest(selection.item.title, selection.library.id, SearchTypes(true, true, false), SearchMethods(true, true, true, false), "", Page(100u, ""), "")).items } else emptyList()
    capture("search", captures, searchItems)
    openDetail(selection, tv)
    capture("detail", captures, listOf(selection.item), detail = true)
}

fun ComposeTestRule.assertHappyPathAndCapture(activity: Activity) = journey(activity, true)

fun ComposeTestRule.assertRegression(activity: Activity) {
    verifyServerContract()
    journey(activity, false)
    val selection = discover()
    openLibrary(selection)
    assertSearchReturn(selection, activity)
    val hierarchy = discoverHierarchy()
    openLibrary(RealSelection(hierarchy.library, hierarchy.series))
    activate(itemCard(hierarchy.series))
    awaitSelectedCard(hierarchy.season)
    activate(itemCard(hierarchy.season))
    waitUntil(15_000) { onAllNodes(hasContentDescription(cardDescription(hierarchy.episode, true)) and hasClickAction()).fetchSemanticsNodes().size == 1 }
    activate(itemCard(hierarchy.episode, true))
    onNode(hasText(hierarchy.episode.title) and hasAnyAncestor(hasTestTag("title-details"))).assertExists()
    onNode(hasContentDescription(hierarchy.series.title) and hasAnyAncestor(hasTestTag("breadcrumbs"))).assertExists()
    activate(onNode(hasContentDescription(hierarchy.season.title) and hasAnyAncestor(hasTestTag("breadcrumbs"))))
    itemCard(hierarchy.episode, true).assertExists()
    activate(onNodeWithContentDescription("Expand sidebar"))
    for (choice in listOf("Subtitles", "Library tracks", "Transcription", "Off")) onNodeWithContentDescription(choice).assertDoesNotExist()
    onNodeWithTag("provider-attribution").assertDoesNotExist()
    activate(onNodeWithContentDescription("Refresh"))
    onNodeWithText("Catalog refreshed.").assertExists()
}

private fun ComposeTestRule.remoteActivate(node: SemanticsNodeInteraction) {
    node.performSemanticsAction(SemanticsActions.RequestFocus) { it() }
    node.assertIsFocused().assertIsDisplayed()
    InstrumentationRegistry.getInstrumentation().sendKeyDownUpSync(KeyEvent.KEYCODE_DPAD_CENTER)
    awaitData()
}

@OptIn(kotlinx.coroutines.ExperimentalCoroutinesApi::class)
fun ComposeTestRule.assertCorrectionEdges(activity: ComponentActivity) {
    val scheduler = kotlinx.coroutines.test.TestCoroutineScheduler()
    val dispatcher = kotlinx.coroutines.test.StandardTestDispatcher(scheduler)
    val state = org.screenroom.ui.PageReadState<Int>()
    runOnUiThread { activity.setContent {
        org.screenroom.ui.ScreenroomTheme {
            var attempt by remember { mutableIntStateOf(0) }
            LaunchedEffect(attempt) {
                kotlinx.coroutines.withContext(dispatcher) {
                    state.load { org.screenroom.ui.readOperation {
                        if (attempt == 0) kotlinx.coroutines.delay(60_000)
                        org.screenroom.ui.collectPages { token ->
                            if (attempt > 0) listOf(3) to ""
                            else if (token.isEmpty()) { kotlinx.coroutines.delay(10_000); listOf(1) to "later" }
                            else { kotlinx.coroutines.delay(60_000); listOf(2) to "" }
                        }
                    } }
                }
            }
            when {
                state.failure != null -> org.screenroom.ui.PageReadError(state.failure!!, "Retry traversal") { attempt++ }
                state.records != null -> androidx.compose.foundation.text.BasicText("Traversal recovered: ${state.records}")
                else -> androidx.compose.foundation.text.BasicText("Loading traversal")
            }
        }
    } }
    waitForIdle()
    runOnIdle { scheduler.runCurrent(); scheduler.advanceTimeBy(120_001); scheduler.runCurrent() }
    onNodeWithText("Page traversal exceeded 120 seconds").assertExists()
    assertEquals(null, state.records)
    onNodeWithContentDescription("Retry traversal").performClick()
    runOnIdle { scheduler.runCurrent() }
    onNodeWithText("Traversal recovered: [3]").assertExists()

    val namespace = argument("SCREENROOM_E2E_SETTINGS_NAMESPACE") + "-failure"
    var fail = true
    val disk = org.screenroom.ui.SettingsStore(activity, namespace) { editor -> if (fail) false else editor.commit() }
    val expected = defaultSettings().copy(source = "jellyfin")
    runBlocking { disk.session.update(expected) }
    assertEquals(expected, disk.session.value)
    assertTrue(disk.session.warning!!.contains("not saved"))
    runOnUiThread { activity.setContent {
        org.screenroom.ui.ScreenroomTheme {
            org.screenroom.ui.HomeScreen(endpoint = argument("SCREENROOM_API_ENDPOINT"), userId = argument("SCREENROOM_USER_ID"), settingsNamespace = namespace, debug = true, settings = disk)
        }
    } }
    awaitHome()
    onNodeWithContentDescription("Retry saving settings").assertExists()
    runOnIdle { fail = false }
    activate(onNodeWithContentDescription("Retry saving settings"))
    waitUntil(5_000) { disk.session.warning == null }
    assertEquals(expected, org.screenroom.ui.SettingsStore(activity, namespace).load())

    val endpoint = mutableStateOf(argument("SCREENROOM_API_ENDPOINT"))
    val user = mutableStateOf(argument("SCREENROOM_USER_ID"))
    val settingsNamespace = mutableStateOf(namespace)
    runOnUiThread { activity.setContent {
        org.screenroom.ui.ScreenroomTheme {
            org.screenroom.ui.HomeScreen(endpoint = endpoint.value, userId = user.value, settingsNamespace = settingsNamespace.value, debug = true)
        }
    } }
    awaitHome()
    val selected = discover()
    openLibrary(selected)
    openDetail(selected, activity.packageManager.hasSystemFeature("android.software.leanback"))
    runOnIdle { user.value = "" }
    waitUntil(15_000) { onAllNodesWithTag("detail-header").fetchSemanticsNodes().isEmpty() }
    onNodeWithTag("search-query").assertDoesNotExist()
    runOnIdle { endpoint.value = "http://127.0.0.1:1" }
    waitUntil(15_000) { onAllNodesWithText("Unable to read catalog. Retry to read server state.").fetchSemanticsNodes().isNotEmpty() }
    onNodeWithTag("home-scope").assertDoesNotExist()
    runOnIdle {
        endpoint.value = argument("SCREENROOM_API_ENDPOINT")
        user.value = argument("SCREENROOM_USER_ID")
        settingsNamespace.value = "$namespace-fresh"
    }
    awaitHome()
    assertEquals(defaultSettings(), org.screenroom.ui.SettingsStore(activity, "$namespace-fresh").load())
}

fun ComposeTestRule.assertSettingsPersistence(activity: Activity) {
    val disk = org.screenroom.ui.SettingsStore(activity, argument("SCREENROOM_E2E_SETTINGS_NAMESPACE"))
    val expected = defaultSettings().copy(source = "jellyfin")
    if (InstrumentationRegistry.getArguments().getString("SCREENROOM_SETTINGS_PHASE") == "write") {
        disk.save(expected)
        assertEquals(expected, disk.load())
        val preferences = activity.getSharedPreferences("screenroom-settings-${argument("SCREENROOM_E2E_SETTINGS_NAMESPACE")}", android.content.Context.MODE_PRIVATE)
        val saved = requireNotNull(preferences.getString("settings", null))
        for (subtitle in listOf(null, "transcription")) {
            val legacy = org.json.JSONObject(saved).put("source", "tmdb")
            if (subtitle != null) legacy.put("subtitle", subtitle)
            check(preferences.edit().putString("settings", legacy.toString()).commit())
            assertEquals(expected.copy(source = "all"), disk.load())
        }
        disk.save(expected)
        try { disk.save(expected.copy(types = SearchTypes(false, false, false))); error("Invalid settings accepted") }
        catch (_: ClientException) { }
        assertEquals(expected, disk.load())
    } else {
        assertEquals(expected, disk.load())
        awaitHome()
        activate(onNodeWithContentDescription("Expand sidebar"))
        onNodeWithContentDescription("Subtitles").assertDoesNotExist()
        onNodeWithContentDescription("Transcription").assertDoesNotExist()
    }
}

fun ComposeTestRule.assertRetry(activity: Activity) {
    val closedPort = java.net.ServerSocket(0, 1, java.net.InetAddress.getByName("127.0.0.1")).use { it.localPort }
    fun intent(endpoint: String) = Intent(activity, activity.javaClass)
        .putExtra("SCREENROOM_API_ENDPOINT", endpoint)
        .putExtra("SCREENROOM_USER_ID", argument("SCREENROOM_USER_ID"))
        .putExtra("SCREENROOM_E2E_SETTINGS_NAMESPACE", argument("SCREENROOM_E2E_SETTINGS_NAMESPACE"))
    androidx.test.core.app.ActivityScenario.launch<ComponentActivity>(intent("http://127.0.0.1:$closedPort")).use {
        repeat(2) { attempt ->
            waitUntil(15_000) { onAllNodesWithText("Unable to read catalog. Retry to read server state.").fetchSemanticsNodes().isNotEmpty() }
            onNodeWithTag("home-scope").assertDoesNotExist()
            onAllNodesWithTag("art-ready", useUnmergedTree = true).assertCountEquals(0)
            if (attempt == 0) activate(onNodeWithContentDescription("Retry"))
        }
    }
    androidx.test.core.app.ActivityScenario.launch<ComponentActivity>(intent(argument("SCREENROOM_API_ENDPOINT"))).use { awaitHome() }
}

private fun verifyServerContract() = withClient { client ->
    var pageReads = 0
    val pageLimited = org.screenroom.ui.PageReadState<Int>()
    pageLimited.load { org.screenroom.ui.collectPages { _ -> pageReads++; listOf(pageReads) to pageReads.toString() } }
    assertEquals(512, pageReads)
    assertEquals("Page limit exceeded", pageLimited.failure)
    assertEquals(null, pageLimited.records)
    val itemLimited = org.screenroom.ui.PageReadState<Int>()
    itemLimited.load { org.screenroom.ui.collectPages { _ -> List(10_001) { it } to "" } }
    assertEquals("Item limit exceeded", itemLimited.failure)
    assertEquals(null, itemLimited.records)
    val repeated = org.screenroom.ui.PageReadState<Int>()
    repeated.load { org.screenroom.ui.collectPages { _ -> listOf(1) to "repeat" } }
    assertEquals("Repeated continuation token", repeated.failure)
    assertEquals(null, repeated.records)
    val info = client.getCatalogInfo()
    assertEquals(info.notices.size, info.notices.map { it.id }.toSet().size)
    assertEquals(info.branding.size, info.branding.map { it.id }.toSet().size)
    val selection = discover()
    val scope = SourceLibraryScope("", selection.library.id)
    val artworks = client.listArtwork(scope, Page(16u, "")).artwork
    assertEquals(artworks.size, artworks.map { it.id }.toSet().size)
    val origin = java.net.URI(argument("SCREENROOM_API_ENDPOINT"))
    for (art in artworks + listOfNotNull(selection.item.artwork)) {
        val url = java.net.URI(art.url)
        assertEquals(origin.scheme, url.scheme); assertEquals(origin.host, url.host); assertEquals(origin.port, url.port)
    }
    assertEquals(selection.item.id, client.getItem(selection.item.id, scope).id)
    val firstCredits = client.listCredits(selection.item.id, Page(16u, ""), scope)
    val credits = firstCredits.credits + if (firstCredits.nextPageToken.isEmpty()) emptyList() else {
        val next = client.listCredits(selection.item.id, Page(16u, firstCredits.nextPageToken), scope)
        assertTrue(next.nextPageToken.isEmpty() || next.nextPageToken != firstCredits.nextPageToken)
        next.credits
    }
    assertEquals(credits.size, credits.map { it.id }.toSet().size)
    val bytes = client.loadArtwork(selection.item.artwork!!.url)
    val bitmap = android.graphics.BitmapFactory.decodeByteArray(bytes, 0, bytes.size)
    assertTrue(bitmap != null && bitmap.width > 0 && bitmap.height > 0)
    bitmap?.recycle()
    assertTrue(client.loadArtwork(selection.item.artwork!!.url).contentEquals(bytes))
    assertEquals(argument("SCREENROOM_USER_ID"), client.getCurrentUser().id)
    try { client.getItem(java.util.UUID.randomUUID().toString(), scope); error("Expected NotFound") }
    catch (_: ClientException.NotFound) { }
}

private fun ComposeTestRule.capture(flow: String, enabled: Boolean, items: List<Item>, detail: Boolean = false) {
    awaitData()
    if (!enabled) return
    val descriptions = items.associateBy { cardDescription(it) }
    waitUntil(15_000) {
        val viewport = onNodeWithTag("content").fetchSemanticsNode().boundsInRoot
        if (detail) {
            onAllNodes(hasTestTag("art-ready") and hasAnyAncestor(hasTestTag("detail-poster")), useUnmergedTree = true).fetchSemanticsNodes().any { it.boundsInRoot.overlaps(viewport) }
        } else {
            val cards = onAllNodes(hasClickAction() and hasAnyAncestor(hasTestTag("content")), useUnmergedTree = true).fetchSemanticsNodes().mapNotNull { card ->
                val description = card.config.getOrNull(SemanticsProperties.ContentDescription)?.singleOrNull() ?: return@mapNotNull null
                descriptions[description]?.let { item -> Triple(card, description, item) }
            }.filter { (card, _, _) -> card.boundsInRoot.width > 0 && card.boundsInRoot.height > 0 && card.boundsInRoot.overlaps(viewport) }
            var visiblePosters = 0
            val ready = cards.all { (card, description, item) ->
                val art = item.artwork
                if (art == null) true else {
                    val widgets = onAllNodes((hasTestTag("art-ready") or hasContentDescription("Artwork unavailable: ${art.label}")) and hasAnyAncestor(hasContentDescription(description) and hasClickAction()), useUnmergedTree = true).fetchSemanticsNodes()
                    val pictures = widgets.filter { picture -> picture.boundsInRoot.width > 0 && picture.boundsInRoot.height > 0 && picture.boundsInRoot.overlaps(viewport) && picture.boundsInRoot.overlaps(card.boundsInRoot) }
                    if (widgets.isEmpty()) false else if (pictures.isEmpty()) true else {
                        visiblePosters++
                        pictures.all { it.config.getOrNull(SemanticsProperties.TestTag) == "art-ready" }
                    }
                }
            }
            ready && visiblePosters > 0
        }
    }
    val instrumentation = InstrumentationRegistry.getInstrumentation()
    instrumentation.uiAutomation.waitForIdle(1_000, 5_000)
    val image = requireNotNull(instrumentation.uiAutomation.takeScreenshot())
    val output = File(instrumentation.targetContext.filesDir, "screenshots/$flow.png")
    assertTrue(output.parentFile!!.isDirectory || output.parentFile!!.mkdirs())
    output.outputStream().use { assertTrue(image.compress(Bitmap.CompressFormat.PNG, 100, it)) }
    image.recycle()
    assertTrue(output.length() > 0)
}
