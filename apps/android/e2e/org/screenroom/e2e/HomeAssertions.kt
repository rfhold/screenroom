package org.screenroom.e2e

import android.app.Activity
import android.content.Intent
import android.graphics.Bitmap
import android.view.KeyEvent
import android.view.WindowInsets
import androidx.activity.ComponentActivity
import androidx.activity.compose.setContent
import androidx.compose.runtime.*
import androidx.compose.ui.semantics.SemanticsActions
import androidx.compose.ui.semantics.SemanticsProperties
import androidx.compose.ui.semantics.getOrNull
import androidx.compose.ui.text.AnnotatedString
import androidx.compose.ui.test.*
import androidx.compose.ui.test.junit4.ComposeTestRule
import androidx.test.platform.app.InstrumentationRegistry
import java.io.File
import org.junit.Assert.assertTrue
import org.junit.Assert.assertEquals
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.runBlocking
import org.screenroom.core.*

private const val bunny = "Big Buck Bunny, Movie, 10m"
fun <A : androidx.activity.ComponentActivity> coreComposeRule(activity: Class<A>, profile: String): androidx.compose.ui.test.junit4.AndroidComposeTestRule<androidx.test.ext.junit.rules.ActivityScenarioRule<A>, A> {
    val instrumentation = InstrumentationRegistry.getInstrumentation()
    val args = InstrumentationRegistry.getArguments()
    val intent = android.content.Intent(instrumentation.targetContext, activity)
        .putExtra("SCREENROOM_API_ENDPOINT", args.getString("SCREENROOM_API_ENDPOINT") ?: "http://127.0.0.1:50051")
        .putExtra("SCREENROOM_USER_ID", args.getString("SCREENROOM_USER_ID") ?: "00000000-0000-0000-0000-000000000001")
        .putExtra("SCREENROOM_E2E_SETTINGS_NAMESPACE", args.getString("SCREENROOM_E2E_SETTINGS_NAMESPACE") ?: profile)
    return androidx.compose.ui.test.junit4.AndroidComposeTestRule(androidx.test.ext.junit.rules.ActivityScenarioRule<A>(intent)) { rule ->
        var current: A? = null
        rule.scenario.onActivity { current = it }
        current!!
    }
}
private const val show = "The Beverly Hillbillies, Series, 1962–1971 · Selected episodes"
private const val season = "Season 1, Season, 2 included episodes"
private const val episode = "The Clampetts Strike Oil, Episode, The Beverly Hillbillies · S1 E1 · ~25m"

@OptIn(kotlinx.coroutines.ExperimentalCoroutinesApi::class)
fun ComposeTestRule.assertCorrectionEdges(activity: ComponentActivity) {
    activeRule = this
    captureEnabled = false
    val scheduler = kotlinx.coroutines.test.TestCoroutineScheduler()
    val dispatcher = kotlinx.coroutines.test.StandardTestDispatcher(scheduler)
    val state = org.screenroom.ui.PageReadState<Int>()
    runOnUiThread {
        activity.setContent {
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
                androidx.compose.foundation.layout.Column {
                    when {
                        state.failure != null -> org.screenroom.ui.PageReadError(state.failure!!, "Retry traversal") { attempt++ }
                        state.records != null -> androidx.compose.foundation.text.BasicText("Traversal recovered: ${state.records}")
                        else -> androidx.compose.foundation.text.BasicText("Loading traversal")
                    }
                }
            }
        }
    }
    waitForIdle()
    runOnIdle { scheduler.runCurrent(); scheduler.advanceTimeBy(120_001); scheduler.runCurrent() }
    onNodeWithText("Page traversal exceeded 120 seconds").assertExists()
    assertEquals(null, state.records)
    onNodeWithContentDescription("Retry traversal").performClick()
    runOnIdle { scheduler.runCurrent() }
    onNodeWithText("Traversal recovered: [3]").assertExists()

    val args = InstrumentationRegistry.getArguments()
    val endpoint = requireNotNull(args.getString("SCREENROOM_API_ENDPOINT"))
    val userId = requireNotNull(args.getString("SCREENROOM_USER_ID"))
    val namespace = requireNotNull(args.getString("SCREENROOM_E2E_SETTINGS_NAMESPACE"))
    val disk = org.screenroom.ui.SettingsStore(activity, namespace)
    disk.save(defaultSettings().copy(scope = SourceLibraryScope("", "tv")))
    val endpointState = mutableStateOf(endpoint)
    val userState = mutableStateOf(userId)
    val namespaceState = mutableStateOf(namespace)
    var back: () -> Boolean = { false }
    runOnUiThread {
        activity.setContent {
            org.screenroom.ui.ScreenroomTheme {
                org.screenroom.ui.HomeScreen(television = activity.packageManager.hasSystemFeature("android.software.leanback"), endpoint = endpointState.value,
                    userId = userState.value, settingsNamespace = namespaceState.value, debug = true, registerBack = { back = it })
            }
        }
    }
    fun awaitScope(value: String) {
        waitUntil(15_000) { onAllNodes(hasTestTag("home-scope") and SemanticsMatcher.expectValue(SemanticsProperties.StateDescription, value)).fetchSemanticsNodes().isNotEmpty() }
    }
    fun awaitSavedLibrary(value: String) { waitUntil(5_000) { disk.load().scope?.libraryId == value } }
    awaitScope("|tv")
    onAllNodesWithContentDescription(bunny).assertCountEquals(0)
    onNode(hasContentDescription("Home") and hasAnyAncestor(hasTestTag("sidebar"))).performClick()
    awaitScope("|")
    onAllNodesWithContentDescription(bunny)[0].assertExists()
    awaitSavedLibrary("")
    onNodeWithContentDescription("TV Shows").performScrollTo().performClick()
    awaitSavedLibrary("tv")
    onNode(hasContentDescription("Movies") and !isToggleable()).performScrollTo().performClick()
    awaitSavedLibrary("movies")
    runOnIdle { assertTrue(back()) }
    awaitData()
    awaitSavedLibrary("tv")
    onNodeWithContentDescription(show).performScrollTo().performClick()
    awaitDetails("The Beverly Hillbillies")
    runOnIdle { userState.value = "" }
    awaitScope("|tv")
    onNodeWithTag("detail-header").assertDoesNotExist()
    onNodeWithTag("search-query").assertDoesNotExist()
    runOnIdle { endpointState.value = "http://127.0.0.1:1" }
    waitUntil(15_000) { onAllNodesWithText("Unable to load catalog:", substring = true).fetchSemanticsNodes().isNotEmpty() }
    onNodeWithTag("home-scope").assertDoesNotExist()
    onNodeWithTag("detail-header").assertDoesNotExist()
    runOnIdle { endpointState.value = endpoint; userState.value = userId; namespaceState.value = "$namespace-new" }
    awaitScope("|")
    onAllNodesWithContentDescription(bunny)[0].assertExists()
    assertEquals("", org.screenroom.ui.SettingsStore(activity, "$namespace-new").load().scope?.libraryId)
    println("Correction edges: own deadline UI Retry/no partial results; TV relaunch/Home/back persisted scopes; same-composition endpoint/user/namespace resets passed")
    assertFinalAssemblyEdges(activity)
}

private fun ComposeTestRule.assertFinalAssemblyEdges(activity: ComponentActivity) {
    val args = InstrumentationRegistry.getArguments()
    val endpoint = requireNotNull(args.getString("SCREENROOM_API_ENDPOINT"))
    val user = requireNotNull(args.getString("SCREENROOM_USER_ID"))
    val namespace = requireNotNull(args.getString("SCREENROOM_E2E_SETTINGS_NAMESPACE")) + "-assembly"
    var failCommit = true
    val disk = org.screenroom.ui.SettingsStore(activity, namespace) { editor -> if (failCommit) false else editor.commit() }
    val expected = ClientSettings(SourceLibraryScope("mock-jellyfin", "tv"), "jellyfin", SearchTypes(false, false, true), SearchMethods(true, true, false, false), "transcription")
    runBlocking { disk.session.update(expected.copy(methods = expected.methods!!.copy(semantic = true))) }
    assertEquals(expected, disk.session.value)
    assertTrue(disk.session.warning!!.contains("not saved"))
    try {
        runBlocking { disk.session.update(expected.copy(types = SearchTypes(false, false, false))) }
        throw AssertionError("Invalid settings accepted")
    } catch (_: ClientException) { }
    assertEquals(expected, disk.session.value)
    runOnUiThread { activity.setContent {
        org.screenroom.ui.ScreenroomTheme { org.screenroom.ui.HomeScreen(endpoint = endpoint, userId = user, settingsNamespace = namespace, debug = true, settings = disk) }
    } }
    waitUntil(15_000) { onAllNodes(hasTestTag("home-scope") and SemanticsMatcher.expectValue(SemanticsProperties.StateDescription, "mock-jellyfin|tv")).fetchSemanticsNodes().isNotEmpty() }
    onNodeWithText("Catalog").assertExists()
    onNodeWithContentDescription("Retry saving settings").performScrollTo().assertExists()
    onNode(hasContentDescription("TV Shows") and hasAnyAncestor(hasTestTag("sidebar"))).performClick()
    awaitData()
    onAllNodesWithContentDescription(show)[0].assertExists()
    onNodeWithContentDescription("Search titles").performClick()
    query("Clampetts")
    awaitResult("1 result in TV Shows")
    assertEquals(expected, disk.session.value)
    runOnIdle { failCommit = false }
    onNodeWithContentDescription("Retry saving settings").performScrollTo().performClick()
    waitUntil(5_000) { disk.session.warning == null }
    assertEquals(expected, org.screenroom.ui.SettingsStore(activity, namespace).load())

    val parentDisk = org.screenroom.ui.SettingsStore(activity, "$namespace-parents")
    parentDisk.save(expected.copy(scope = SourceLibraryScope("mock-jellyfin", "movies")))
    runOnUiThread { activity.setContent {
        org.screenroom.ui.ScreenroomTheme { org.screenroom.ui.HomeScreen(endpoint = endpoint, userId = user, settingsNamespace = "$namespace-parents", debug = true, settings = parentDisk) }
    } }
    waitUntil(15_000) { onAllNodes(hasTestTag("home-scope") and SemanticsMatcher.expectValue(SemanticsProperties.StateDescription, "mock-jellyfin|movies")).fetchSemanticsNodes().isNotEmpty() }
    onAllNodesWithContentDescription(bunny)[0].assertExists()
    onAllNodesWithContentDescription(show).assertCountEquals(0)
    onNode(hasContentDescription("TV Shows") and hasAnyAncestor(hasTestTag("sidebar"))).performClick()
    awaitData()
    onNodeWithContentDescription("Search titles").performClick()
    query("Clampetts")
    awaitResult("1 result in TV Shows")
    onNodeWithContentDescription(episode).performScrollTo().performClick()
    waitUntil(15_000) { onAllNodes(hasContentDescription("Season 1") and hasAnyAncestor(hasTestTag("breadcrumbs"))).fetchSemanticsNodes().isNotEmpty() }
    onNode(hasContentDescription("The Beverly Hillbillies") and hasAnyAncestor(hasTestTag("breadcrumbs"))).assertExists()
    onNode(hasText("The Clampetts Strike Oil") and hasAnyAncestor(hasTestTag("detail-header"))).assertExists()
    assertEquals(SourceLibraryScope("mock-jellyfin", "tv"), parentDisk.session.value.scope)
    println("Final assembly: false commit retains all normalized choices; healthy Home/search, invalid-input rejection and actual Retry save; episode from Search with movie-only Home cache loads scoped season/series breadcrumbs")
}

fun ComposeTestRule.assertSettingsPersistence(activity: Activity) {
    activeRule = this
    val args = InstrumentationRegistry.getArguments()
    val namespace = requireNotNull(args.getString("SCREENROOM_E2E_SETTINGS_NAMESPACE"))
    val disk = org.screenroom.ui.SettingsStore(activity, namespace)
    val expected = ClientSettings(SourceLibraryScope("mock-jellyfin", "tv"), "jellyfin", SearchTypes(false, true, false), SearchMethods(true, true, false, false), "transcription")
    if (args.getString("SCREENROOM_SETTINGS_PHASE") == "write") {
        disk.save(expected)
        assertEquals(expected, org.screenroom.ui.SettingsStore(activity, namespace).load())
        try {
            disk.save(expected.copy(types = SearchTypes(false, false, false)))
            throw AssertionError("Empty search types must be rejected")
        } catch (_: ClientException) { }
        assertEquals(expected, disk.load())
        assertEquals(defaultSettings(), org.screenroom.ui.SettingsStore(activity, "$namespace-isolated").load())
        activity.getSharedPreferences("screenroom-settings-$namespace-invalid", android.content.Context.MODE_PRIVATE).edit().putString("settings", "{invalid").commit()
        assertEquals(defaultSettings(), org.screenroom.ui.SettingsStore(activity, "$namespace-invalid").load())
    } else {
        assertEquals(expected, disk.load())
        waitUntil(15_000) { onAllNodesWithText("Continue watching").fetchSemanticsNodes().isNotEmpty() }
        onNodeWithContentDescription("Search titles").performClick()
        onNodeWithContentDescription("Search settings").performClick()
        onNodeWithContentDescription("Jellyfin").assertIsOn()
        onNode(hasContentDescription("Movies") and isToggleable()).assertIsOff()
        onNode(hasContentDescription("Shows") and isToggleable()).assertIsOn()
        onNodeWithContentDescription("Title only").performScrollTo().assertIsOn()
        onNodeWithContentDescription("Semantic").performScrollTo().assertIsOff()
        onNodeWithContentDescription("Search settings").performScrollTo().performClick()
        query("Beverly")
        awaitResult("1 result in TV Shows")
        onNodeWithTag("search-effective-methods").assertTextEquals("Effective search: Text")
        println("Durable settings after process relaunch: namespace=$namespace scope/source/types/methods/subtitle preserved")
        onNode(hasContentDescription("Home") and hasAnyAncestor(hasTestTag("sidebar"))).performClick()
        waitUntil(15_000) { onAllNodes(hasTestTag("home-scope") and SemanticsMatcher.expectValue(SemanticsProperties.StateDescription, "mock-jellyfin|")).fetchSemanticsNodes().isNotEmpty() }
        onAllNodesWithContentDescription(bunny)[0].assertExists()
        waitUntil(5_000) { disk.load().scope?.libraryId == "" }
        assertEquals(expected.copy(scope = SourceLibraryScope("mock-jellyfin", "")), disk.load())
        println("Persisted TV process relaunch -> Home: unscoped library request/actual movie rows/local scope cleared, other settings retained")
    }
}

fun ComposeTestRule.assertRegression(activity: Activity) {
    activeRule = this
    captureEnabled = false
    waitUntil(15_000) { onAllNodesWithText("Continue watching").fetchSemanticsNodes().isNotEmpty() }
    waitUntil(15_000) { onAllNodesWithTag("art-ready", useUnmergedTree = true).fetchSemanticsNodes().isNotEmpty() }
    val instrumentation = InstrumentationRegistry.getInstrumentation()
    val tv = instrumentation.targetContext.packageManager.hasSystemFeature("android.software.leanback")
    verifyServerContract(if (tv) "tv" else "mobile")
    if (tv) assertTvSidebarRemote(activity)
    onNodeWithText("Home").assertIsDisplayed()
    onNodeWithText("Continue watching").assertIsDisplayed()
    onAllNodesWithContentDescription("Big Buck Bunny poster", useUnmergedTree = true)[0].assertExists()
    if (instrumentation.targetContext.packageManager.hasSystemFeature("android.software.leanback")) {
        onAllNodesWithContentDescription("Tears of Steel, Movie, 12m 14s")[0].performSemanticsAction(SemanticsActions.RequestFocus) { it() }
        onAllNodesWithContentDescription("Tears of Steel, Movie, 12m 14s")[0].assertIsFocused().assertIsDisplayed()
        instrumentation.sendKeyDownUpSync(KeyEvent.KEYCODE_DPAD_CENTER)
        awaitDetails("Tears of Steel")
        onNodeWithText("Movie · 2012").assertIsDisplayed()
        onNodeWithText("12m 14s", substring = true).assertExists()
        instrumentation.sendKeyDownUpSync(KeyEvent.KEYCODE_BACK)
        waitForIdle()
    }
    onNodeWithText("Catalog").performScrollTo().assertIsDisplayed()
    val collapsed = onNodeWithTag("content").fetchSemanticsNode().boundsInRoot
    val toggle = onNodeWithContentDescription("Expand sidebar").fetchSemanticsNode().boundsInRoot
    onNodeWithContentDescription("Expand sidebar").performClick()
    val expanded = onNodeWithTag("content").fetchSemanticsNode().boundsInRoot
    assertTrue(expanded.left > collapsed.left || expanded.top > collapsed.top)
    assertTrue(onNodeWithContentDescription("Collapse sidebar").fetchSemanticsNode().boundsInRoot.topLeft == toggle.topLeft)
    onNodeWithText("Libraries").assertIsDisplayed()
    if (tv) onNodeWithContentDescription("Subtitles").performClick()
    onNodeWithContentDescription("Transcription").scrollOnMobile(tv).performClick()
    onNodeWithContentDescription("Refresh").scrollOnMobile(tv).performClick()
    onNodeWithText("Catalog refreshed.").scrollOnMobile(tv).assertIsDisplayed()
    onNodeWithContentDescription("TV Shows").performScrollTo().performClick()
    onNodeWithContentDescription(show).performClick()
    awaitDetails("The Beverly Hillbillies")
    onNodeWithText("Title rights").assertExists()
    onNodeWithContentDescription(season).performScrollTo().performClick()
    awaitDetails("Season 1")
    onNodeWithText("Selected episodes from the 36-episode first season; not the complete season.").assertExists()
    onNodeWithContentDescription("TMDB · 6.1/10").assertExists().assertHasClickAction()
    onNodeWithContentDescription("Rating and catalog details").performScrollTo().performClick()
    onNodeWithText("TMDB user rating: 6.1/10 · Vote count unavailable.", substring = true).assertExists()
    onNodeWithText("TMDB rating snapshot not imported.").assertDoesNotExist()
    onNodeWithText("TMDB · Listed").assertExists()
    onAllNodesWithText("Jellyfin · Not connected")[0].assertExists()
    onNodeWithText("0 votes", substring = true).assertDoesNotExist()
    onNodeWithText("Unrated", substring = true).assertDoesNotExist()
    onNodeWithContentDescription(episode).performScrollTo().performClick()
    awaitDetails("The Clampetts Strike Oil")
    onNodeWithTag("title-details").performScrollTo().assertIsDisplayed()
    onNodeWithText("Play", substring = false).assertDoesNotExist()
    onNodeWithText("The Beverly Hillbillies · S1 E1 · ~25m · Aired 1962-09-26").assertExists()
    onNodeWithContentDescription("Rating and catalog details").performScrollTo().performClick()
    onNodeWithText("Snapshot · 2026-11-07 · Not live.", substring = true).assertExists()
    onNodeWithText("Catalog membership is not playability.").assertExists()
    onNodeWithText("Only S01E01 and S01E02 are included.", substring = true).assertExists()
    onNodeWithContentDescription("The Beverly Hillbillies").performScrollTo().performClick()
    awaitDetails("The Beverly Hillbillies")
    onNodeWithContentDescription(season).assertExists()
    onNodeWithText("Season 2").assertDoesNotExist()
    onNodeWithContentDescription("Search titles").performClick()
    onNodeWithText("Catalog").assertDoesNotExist()
    onNode(hasContentDescription("Shows") and isToggleable()).assertDoesNotExist()
    onNodeWithContentDescription("Search settings").performClick()
    onNode(hasContentDescription("Episodes") and isToggleable()).performScrollTo().performClick()
    val movieType = onNode(hasContentDescription("Movies") and isToggleable())
    val showType = onNode(hasContentDescription("Shows") and isToggleable())
    movieType.performScrollTo().performClick()
    showType.assertIsNotEnabled()
    movieType.performClick()
    onNodeWithContentDescription("Search settings").performScrollTo().performClick()
    query("Beverli")
    awaitResult("1 result in TV Shows")
    query("wealth")
    awaitResult("1 result in TV Shows")
    onNodeWithContentDescription("Search settings").performScrollTo().performClick()
    onNodeWithContentDescription("Title only").performScrollTo().performClick()
    onNodeWithContentDescription("Semantic").performScrollTo().assertIsNotEnabled()
    onNodeWithContentDescription("Search settings").performScrollTo().performClick()
    awaitResult("No matches in TV Shows. Try another search.")
    query("Beverly")
    awaitResult("1 result in TV Shows")
    onNodeWithContentDescription(show).performScrollTo().performClick()
    awaitDetails("The Beverly Hillbillies")
    onNodeWithContentDescription("Search results").performScrollTo().performClick()
    onNodeWithTag("search-query").assertTextEquals("Beverly")
    onNodeWithContentDescription("Search settings").performClick()
    onNodeWithContentDescription("Title only").performScrollTo().assertIsOn()
    onNodeWithContentDescription("Fuzzy titles").performScrollTo().performClick()
    onNodeWithContentDescription("Text (titles only)").assertIsNotEnabled()
    onNodeWithContentDescription("Jellyfin").performScrollTo().performClick()
    onNodeWithContentDescription("Search settings").performScrollTo().performClick()
    awaitResult("1 result in TV Shows")
    onNodeWithContentDescription("Search settings").performClick()
    onNodeWithContentDescription("TMDB").performScrollTo().performClick()
    onNodeWithContentDescription("Search settings").performScrollTo().performClick()
    awaitResult("No matches in TV Shows. Try another search.")
    onNodeWithContentDescription("Search settings").performClick()
    onNodeWithContentDescription("Jellyfin").performScrollTo().performClick()
    onNodeWithContentDescription("Search settings").performScrollTo().performClick()
    awaitResult("1 result in TV Shows")
    onNodeWithContentDescription(show).performScrollTo().performClick()
    awaitDetails("The Beverly Hillbillies")
    onNodeWithContentDescription("Search titles").performClick()
    onNodeWithTag("search-query").assert(SemanticsMatcher.expectValue(SemanticsProperties.EditableText, AnnotatedString("")))
    onNodeWithContentDescription("Search settings").performClick()
    onNodeWithContentDescription("Jellyfin").assertIsOn()
    onNodeWithContentDescription("Title only").performScrollTo().assertIsOn()
    onNodeWithContentDescription("Semantic").performScrollTo().assertIsOff()
    onNodeWithContentDescription("Search settings").performScrollTo().performClick()
    query("Clampetts")
    dismissKeyboard(activity)
    instrumentation.sendKeyDownUpSync(KeyEvent.KEYCODE_BACK)
    awaitDetails("The Beverly Hillbillies")
    onNodeWithContentDescription(season).assertExists()
    instrumentation.sendKeyDownUpSync(KeyEvent.KEYCODE_BACK)
    waitForIdle()
    onNodeWithTag("search-query").assertTextEquals("Beverly")
    onNodeWithContentDescription("Search settings").performClick()
    onNodeWithContentDescription("Jellyfin").assertIsOn()
    onNodeWithContentDescription("Title only").performScrollTo().assertIsOn()
    onNodeWithContentDescription("Fuzzy titles").performScrollTo().assertIsOff()
    onNodeWithContentDescription("Search settings").performScrollTo().performClick()
    onNodeWithContentDescription("Clear query").performClick()
    onNodeWithText("Enter a title to search TV Shows.").assertExists()
    onNode(hasContentDescription("Movies") and !isToggleable()).performClick()
    onNodeWithContentDescription(bunny).assertExists()
    onNodeWithContentDescription(show).assertDoesNotExist()
    onNodeWithContentDescription("Search titles").performClick()
    query("Beverly")
    awaitResult("No matches in Movies. Try another search.")
    onNodeWithContentDescription("Expand sidebar").performClick()
    onNodeWithContentDescription("Log out").assertDoesNotExist()
    onNodeWithText("Welcome").assertDoesNotExist()
    onNodeWithContentDescription("Continue").assertDoesNotExist()
    onNodeWithContentDescription("Refresh").scrollOnMobile(tv).performClick()
    onNodeWithText("Catalog refreshed.").assertExists()
    onNode(hasContentDescription("Home") and hasAnyAncestor(hasTestTag("sidebar"))).performClick()
    onNodeWithContentDescription("Expand sidebar").performClick()
    if (tv) onNodeWithContentDescription("Subtitles").performClick()
    onNodeWithContentDescription("Off").scrollOnMobile(tv).performClick()
    if (!tv) onNodeWithContentDescription("Off").assert(SemanticsMatcher.expectValue(SemanticsProperties.StateDescription, "Selected"))
    assertLocalSubtitleOff()
    onNodeWithContentDescription("Collapse sidebar").performClick()
    onNodeWithText("Screenroom").assertDoesNotExist()
    captureShowcase("home") { onNodeWithContentDescription("Expand sidebar").assertIsDisplayed() }
    if (tv) {
        instrumentation.sendKeyDownUpSync(KeyEvent.KEYCODE_DPAD_LEFT)
        waitForIdle()
        onNodeWithContentDescription("Collapse sidebar").assertIsFocused()
    } else onNodeWithContentDescription("Expand sidebar").performClick()
    onNodeWithContentDescription("Collapse sidebar").assertIsDisplayed()
    onNodeWithText("Libraries").assertIsDisplayed()
    captureShowcase("sidebar") {
        if (!tv) onNodeWithContentDescription("Off").assert(SemanticsMatcher.expectValue(SemanticsProperties.StateDescription, "Selected"))
        if (tv) {
            onNodeWithContentDescription("Home").assertIsSelected()
            onNodeWithContentDescription("Subtitles").assert(SemanticsMatcher.expectValue(SemanticsProperties.StateDescription, "Off, Collapsed"))
            onNodeWithTag("subtitle-choices").assertDoesNotExist()
            onNodeWithContentDescription("Refresh").assertIsDisplayed()
            onNodeWithContentDescription("Log out").assertDoesNotExist()
            onNodeWithContentDescription("Collapse sidebar").assertIsFocused()
        }
    }
    onNodeWithContentDescription("Collapse sidebar").performClick()
    onNode(hasContentDescription("Movies") and !isToggleable()).performScrollTo().performClick()
    onNodeWithContentDescription("Nosferatu, Movie, 63–94m · version dependent").assertIsDisplayed()
    onNodeWithContentDescription("His Girl Friday theatrical poster (1939)", useUnmergedTree = true).assertExists()
    captureShowcase("library")
    onNodeWithContentDescription("All libraries").performScrollTo().performClick()
    onNodeWithContentDescription("Search titles").performClick()
    query("Bunny")
    awaitResult("1 result in All libraries")
    dismissKeyboard(activity)
    captureShowcase("search") {
        onNodeWithTag("search-query").assertIsNotFocused().assertTextEquals("Bunny")
        onNodeWithText("1 result in All libraries").assertIsDisplayed()
        onNodeWithText("Catalog").assertDoesNotExist()
        onNodeWithContentDescription(bunny).assertIsDisplayed()
        assertTrue(!activity.window.decorView.rootWindowInsets.isVisible(WindowInsets.Type.ime()))
    }
    onNode(hasContentDescription("Movies") and !isToggleable()).performScrollTo().performClick()
    onNodeWithContentDescription(bunny).performScrollTo().performClick()
    awaitDetails("Big Buck Bunny")
    onNodeWithContentDescription("Jellyfin: Not connected").assertExists()
    onNodeWithText("Play", substring = false).assertDoesNotExist()
    onNodeWithContentDescription("Big Buck Bunny poster", useUnmergedTree = true).assertIsDisplayed()
    onAllNodesWithText("Sacha Goedegebure").assertCountEquals(2)
    onAllNodesWithContentDescription("Sacha Goedegebure portrait", useUnmergedTree = true).assertCountEquals(2)
    onNodeWithText("CC BY 3.0").assertExists()
    onNodeWithContentDescription("Search results").assertDoesNotExist()
    assertDetailGeometry(activity)
    captureShowcase("detail")
    onNodeWithContentDescription("Data & artwork credits").performScrollTo().performClick()
    onNode(hasText("This product uses the TMDB API but is not endorsed or certified by TMDB.") and hasTestTag("catalog-notice-tmdb-attribution")).assertExists()
    onNodeWithContentDescription("TMDB approved primary short blue logo").assertExists()
    waitUntil(15_000) { onAllNodes(SemanticsMatcher("artwork credits") { it.config.getOrNull(SemanticsProperties.TestTag)?.startsWith("artwork-credit-") == true }).fetchSemanticsNodes().size == 21 }
    waitUntil(15_000) { onAllNodes(hasContentDescription("TMDB approved primary short blue logo") and hasTestTag("art-ready"), useUnmergedTree = true).fetchSemanticsNodes().size == 1 }
    onNodeWithText("TMDB lists the same film as 2024-04-06", substring = true).assertExists()
    onAllNodesWithText("William Maanders")[0].assertExists()
    androidx.test.core.app.ActivityScenario.launch<ComponentActivity>(
        Intent(activity, activity.javaClass).putExtra("SCREENROOM_API_ENDPOINT", "http://127.0.0.2:50051")
    ).use {
        waitUntil(5_000) { onAllNodesWithText("API configuration error").fetchSemanticsNodes().isNotEmpty() }
        onNodeWithText("API configuration error").assertIsDisplayed()
        onNodeWithText("HTTPS API endpoint required. Debug HTTP is limited to 127.0.0.1 and localhost.").assertIsDisplayed()
    }
}

fun ComposeTestRule.assertHappyPathAndCapture(activity: Activity) {
    activeRule = this
    captureEnabled = true
    val instrumentation = InstrumentationRegistry.getInstrumentation()
    val tv = activity.packageManager.hasSystemFeature("android.software.leanback")
    waitUntil(15_000) { onAllNodesWithText("Continue watching").fetchSemanticsNodes().isNotEmpty() }
    onNodeWithText("Home").assertIsDisplayed()
    captureShowcase("home") { onNodeWithContentDescription("Expand sidebar").assertIsDisplayed() }
    if (tv) {
        onNodeWithContentDescription("Browse libraries").assertIsFocused()
        instrumentation.sendKeyDownUpSync(KeyEvent.KEYCODE_DPAD_LEFT)
        waitForIdle()
        onNodeWithContentDescription("Collapse sidebar").assertIsFocused()
    } else onNodeWithContentDescription("Expand sidebar").performClick()
    captureShowcase("sidebar") {
        onNodeWithContentDescription("Collapse sidebar").assertIsDisplayed()
        if (tv) {
            onNodeWithContentDescription("Home").assertIsSelected()
            onNodeWithContentDescription("Collapse sidebar").assertIsFocused()
            onNodeWithContentDescription("Subtitles").assert(SemanticsMatcher.expectValue(SemanticsProperties.StateDescription, "Off, Collapsed"))
            onNodeWithTag("subtitle-choices").assertDoesNotExist()
            onNodeWithContentDescription("Refresh").assertIsDisplayed()
            onNodeWithContentDescription("Log out").assertDoesNotExist()
            val sidebar = onNodeWithTag("sidebar").fetchSemanticsNode().boundsInRoot
            val content = onNodeWithTag("content").fetchSemanticsNode().boundsInRoot
            assertTrue(sidebar.width / (sidebar.width + content.width) in .18f.. .20f)
        } else onNodeWithContentDescription("Off").assert(SemanticsMatcher.expectValue(SemanticsProperties.StateDescription, "Selected"))
    }
    assertLocalSubtitleOff()
    if (tv) {
        repeat(4) { instrumentation.sendKeyDownUpSync(KeyEvent.KEYCODE_DPAD_DOWN); waitForIdle() }
        onNodeWithContentDescription("Movies").assertIsFocused().assertIsNotSelected()
        instrumentation.sendKeyDownUpSync(KeyEvent.KEYCODE_DPAD_CENTER)
    } else {
        onNodeWithContentDescription("Collapse sidebar").performClick()
        onNode(hasContentDescription("Movies") and !isToggleable()).performClick()
    }
    waitUntil(15_000) { onAllNodesWithContentDescription("Nosferatu, Movie, 63–94m · version dependent").fetchSemanticsNodes().isNotEmpty() }
    if (tv) onNodeWithContentDescription("Nosferatu, Movie, 63–94m · version dependent").assertIsFocused()
    captureShowcase("library") {
        onNodeWithContentDescription("Expand sidebar").assertIsDisplayed()
        onNode(hasText("Movies") and !hasClickAction()).assertIsDisplayed()
    }
    onNodeWithContentDescription("All libraries").performScrollTo().performClick()
    onNodeWithContentDescription("Search titles").performClick()
    onNodeWithContentDescription("Search settings").performClick()
    onNodeWithContentDescription("All").assertIsOn()
    listOf("Movies", "Shows").forEach {
        onNode(hasContentDescription(it) and isToggleable()).assertIsOn()
    }
    onNode(hasContentDescription("Episodes") and isToggleable()).assertIsOn()
    onNodeWithContentDescription("Semantic").performScrollTo().performClick()
    onNodeWithContentDescription("Search settings").performScrollTo().performClick()
    if (tv) onNodeWithTag("search-query").performClick()
    query("Bunny")
    awaitResult("1 result in All libraries")
    dismissKeyboard(activity, nativeFocus = tv)
    captureShowcase("search") {
        onNodeWithTag("search-query").assertIsNotFocused().assertTextEquals("Bunny")
        onNodeWithText("1 result in All libraries").assertIsDisplayed()
        onNodeWithContentDescription(bunny).assertIsDisplayed()
        onNodeWithText("Catalog").assertDoesNotExist()
        assertTrue(!activity.window.decorView.rootWindowInsets.isVisible(WindowInsets.Type.ime()))
    }
    onNode(hasContentDescription("Movies") and !isToggleable()).performScrollTo().performClick()
    waitUntil(15_000) { onAllNodesWithContentDescription(bunny).fetchSemanticsNodes().isNotEmpty() }
    if (tv) {
        val firstMovie = "Nosferatu, Movie, 63–94m · version dependent"
        waitUntil(15_000) { onAllNodes(hasContentDescription(firstMovie) and isFocused()).fetchSemanticsNodes().size == 1 }
        onNodeWithContentDescription(firstMovie).assertIsFocused().assertIsDisplayed()
        instrumentation.sendKeyDownUpSync(KeyEvent.KEYCODE_DPAD_DOWN)
        waitForIdle()
        onNodeWithContentDescription("Night of the Living Dead, Movie, 1h 36m").assertIsFocused().assertIsDisplayed()
        instrumentation.sendKeyDownUpSync(KeyEvent.KEYCODE_DPAD_RIGHT)
        waitForIdle()
        onNodeWithContentDescription(bunny).assertIsFocused().assertIsDisplayed()
        instrumentation.sendKeyDownUpSync(KeyEvent.KEYCODE_DPAD_CENTER)
    } else onNodeWithContentDescription(bunny).performScrollTo().performClick()
    awaitDetails("Big Buck Bunny")
    assertDetailGeometry(activity)
    captureShowcase("detail") {
        onNodeWithContentDescription("Expand sidebar").assertIsDisplayed()
        onNodeWithContentDescription("Search results").assertDoesNotExist()
        onNodeWithText("Movies").assertIsDisplayed()
        onNodeWithContentDescription("Big Buck Bunny poster", useUnmergedTree = true).assertIsDisplayed()
        onNodeWithContentDescription("Jellyfin: Not connected").assertExists()
        onNodeWithText("Play", substring = false).assertDoesNotExist()
        onNodeWithText("TMDB user rating:", substring = true).assertDoesNotExist()
    }
}

fun ComposeTestRule.assertRetry(activity: Activity) {
    activeRule = this
    captureEnabled = false
    val args = InstrumentationRegistry.getArguments()
    val endpoint = args.getString("SCREENROOM_API_ENDPOINT") ?: "http://127.0.0.1:50051"
    val namespace = args.getString("SCREENROOM_E2E_SETTINGS_NAMESPACE") ?: "android-retry"
    val userId = args.getString("SCREENROOM_USER_ID") ?: "00000000-0000-0000-0000-000000000001"
    val closedPort = java.net.ServerSocket(0, 1, java.net.InetAddress.getByName("127.0.0.1")).use { it.localPort }
    fun intent(url: String) = Intent(activity, activity.javaClass)
        .putExtra("SCREENROOM_API_ENDPOINT", url).putExtra("SCREENROOM_USER_ID", userId)
        .putExtra("SCREENROOM_E2E_SETTINGS_NAMESPACE", namespace)
    androidx.test.core.app.ActivityScenario.launch<ComponentActivity>(intent("http://127.0.0.1:$closedPort")).use {
        fun assertUnavailable() {
            waitUntil(15_000) { onAllNodesWithText("Unable to load catalog:", substring = true).fetchSemanticsNodes().isNotEmpty() }
            onNodeWithText("Unable to load catalog:", substring = true).assertIsDisplayed()
            onNodeWithContentDescription("Retry").assertIsDisplayed()
            onNodeWithText("Continue watching").assertDoesNotExist()
            onAllNodesWithTag("art-ready", useUnmergedTree = true).assertCountEquals(0)
            onAllNodesWithContentDescription(bunny).assertCountEquals(0)
        }
        assertUnavailable()
        if (activity.packageManager.hasSystemFeature("android.software.leanback")) {
            onNodeWithContentDescription("Retry").performSemanticsAction(SemanticsActions.RequestFocus) { it() }
            onNodeWithContentDescription("Retry").assertIsFocused()
            InstrumentationRegistry.getInstrumentation().sendKeyDownUpSync(KeyEvent.KEYCODE_DPAD_CENTER)
            awaitData()
        } else onNodeWithContentDescription("Retry").performClick()
        assertUnavailable()
    }
    androidx.test.core.app.ActivityScenario.launch<ComponentActivity>(intent(endpoint)).use {
        waitUntil(15_000) { onAllNodesWithText("Continue watching").fetchSemanticsNodes().isNotEmpty() }
        waitUntil(15_000) { onAllNodesWithTag("art-ready", useUnmergedTree = true).fetchSemanticsNodes().isNotEmpty() }
        onNodeWithText("Continue watching").assertIsDisplayed()
        onAllNodesWithContentDescription(bunny)[0].assertIsDisplayed()
        onNodeWithText("Unable to load catalog:", substring = true).assertDoesNotExist()
        onNodeWithContentDescription("Expand sidebar").performClick()
        val tv = activity.packageManager.hasSystemFeature("android.software.leanback")
        if (tv) onNodeWithContentDescription("Subtitles").performClick()
        onNodeWithContentDescription("Transcription").scrollOnMobile(tv).performClick()
    }
    androidx.test.core.app.ActivityScenario.launch<ComponentActivity>(intent(endpoint)).use {
        waitUntil(15_000) { onAllNodesWithText("Continue watching").fetchSemanticsNodes().isNotEmpty() }
        onNodeWithContentDescription("Expand sidebar").performClick()
        if (activity.packageManager.hasSystemFeature("android.software.leanback")) {
            onNodeWithContentDescription("Subtitles").assert(SemanticsMatcher.expectValue(SemanticsProperties.StateDescription, "Transcription, Collapsed"))
        } else onNodeWithContentDescription("Transcription").assert(SemanticsMatcher.expectValue(SemanticsProperties.StateDescription, "Selected"))
        assertEquals("transcription", org.screenroom.ui.SettingsStore(activity, namespace).load().subtitlePreference)
    }
    println("Retry: actual closed-port error and Retry remain unavailable; valid-endpoint Activity relaunch restores real Home/artwork (not in-place server restart)")
}

private fun ComposeTestRule.assertDetailGeometry(activity: Activity) {
    val density = activity.resources.displayMetrics.density
    val header = onNodeWithTag("detail-header").fetchSemanticsNode().boundsInRoot
    val poster = onNodeWithTag("detail-poster").fetchSemanticsNode().boundsInRoot
    val summary = onNodeWithTag("title-details").fetchSemanticsNode().boundsInRoot
    val breadcrumbs = onNodeWithTag("breadcrumbs").fetchSemanticsNode().boundsInRoot
    val cast = onNodeWithTag("people-Cast").fetchSemanticsNode().boundsInRoot
    assertTrue(breadcrumbs.height <= 50 * density)
    assertTrue(kotlin.math.abs(poster.height / poster.width - 1.5f) < .02f)
    if (header.width / density >= 640) {
        assertTrue(summary.left >= poster.right)
        assertTrue(summary.top == poster.top)
        assertTrue(poster.height <= 300 * density)
    } else {
        assertTrue(summary.top >= poster.bottom)
        assertTrue(kotlin.math.abs(poster.width / density - 160) < 1)
    }
    assertTrue(cast.top >= header.bottom)
    assertTrue(cast.left == header.left && cast.width >= header.width - 1)
}

private fun assertLocalSubtitleOff() {
    val context = InstrumentationRegistry.getInstrumentation().targetContext
    val namespace = InstrumentationRegistry.getArguments().getString("SCREENROOM_E2E_SETTINGS_NAMESPACE") ?: "default"
    assertEquals("off", org.screenroom.ui.SettingsStore(context, namespace).load().subtitlePreference)
}

private fun verifyServerContract(platform: String) = runBlocking(Dispatchers.IO) {
    val endpoint = InstrumentationRegistry.getArguments().getString("SCREENROOM_API_ENDPOINT") ?: "http://127.0.0.1:50051"
    val userId = InstrumentationRegistry.getArguments().getString("SCREENROOM_USER_ID") ?: "00000000-0000-0000-0000-000000000001"
    val config = ClientConfig(endpoint, userId, 3000u)
    val client = Client(config)
    try {
        val catalogInfo = client.getCatalogInfo()
        val artworks = org.screenroom.ui.collectPages { token ->
            val page = client.listArtwork(SourceLibraryScope("", ""), Page(2u, token))
            page.artwork to page.nextPageToken
        }
        assertEquals(21, artworks.size)
        assertEquals(21, artworks.map { it.id }.toSet().size)
        assertEquals(7, catalogInfo.notices.size)
        assertEquals(listOf("tmdb-attribution", "tmdb-terms", "mock-catalog", "server-artwork", "sprite-fright-identity", "edition-rights", "included-episodes"), catalogInfo.notices.map { it.id })
        val origin = java.net.URI(endpoint)
        artworks.forEach { art ->
            val url = java.net.URI(art.url)
            assertEquals(origin.scheme, url.scheme)
            assertEquals(origin.host, url.host)
            assertEquals(origin.port, url.port)
            assertTrue(art.creator.isNotBlank() && art.license.isNotBlank() && art.source.isNotBlank())
        }
        val logo = catalogInfo.branding.single { it.kind == "logo" }
        val logoBytes = client.loadArtwork(logo.url)
        assertTrue(logoBytes.take(8).map { it.toInt() and 255 } == listOf(137, 80, 78, 71, 13, 10, 26, 10))
        val logoBitmap = android.graphics.BitmapFactory.decodeByteArray(logoBytes, 0, logoBytes.size)
        assertTrue(logoBitmap != null && logoBitmap.width > 0 && logoBitmap.height > 0)
        println("CatalogInfo RPC: $platform artwork=${artworks.size} notices=${catalogInfo.notices.size} logo=${logo.url} bytes=${logoBytes.size} bitmap=${logoBitmap?.width}x${logoBitmap?.height} sha256=${java.security.MessageDigest.getInstance("SHA-256").digest(logoBytes).joinToString("") { "%02x".format(it) }}")
        logoBitmap?.recycle()
        val home = client.getHome(SourceLibraryScope("", ""))
        assertTrue(home.isNotEmpty())
        val movies = org.screenroom.ui.collectPages { token ->
            val result = client.listLibraryItems(SourceLibraryScope("", "movies"), Page(1u, token))
            result.items to result.nextPageToken
        }
        assertEquals(8, movies.size)
        val bunnyItem = movies.single { it.title == "Big Buck Bunny" }
        assertEquals(bunnyItem.id, client.getItem(bunnyItem.id, SourceLibraryScope("", "movies")).id)
        val credits = org.screenroom.ui.collectPages { token ->
            val result = client.listCredits(bunnyItem.id, Page(1u, token), SourceLibraryScope("", "movies"))
            result.credits to result.nextPageToken
        }
        assertTrue(credits.size > 1)
        val search = client.search(SearchRequest("Bunny", "", SearchTypes(true, true, true), SearchMethods(false, true, true, true), "all", Page(100u, ""), ""))
        assertTrue(search.items.any { it.id == bunnyItem.id })
        assertTrue(search.effectiveMethods?.semantic == true || search.degradationReason.isNotBlank())
        val art = client.loadArtwork(bunnyItem.artwork!!.url)
        val bitmap = android.graphics.BitmapFactory.decodeByteArray(art, 0, art.size)
        assertTrue(bitmap != null && bitmap.width > 0)
        bitmap?.recycle()
        var artworkFailed = false
        try { client.loadArtwork(bunnyItem.artwork!!.url.replace("/revisions/1/", "/revisions/2147483647/")) }
        catch (_: ClientException) { artworkFailed = true }
        assertTrue("A missing artwork revision must not return a bundled/provider fallback", artworkFailed)
        assertTrue(client.loadArtwork(bunnyItem.artwork!!.url).contentEquals(art))
        assertEquals(userId, client.getCurrentUser().id)
        try { client.getItem("00000000-0000-0000-0000-000000000099", SourceLibraryScope("", "")); throw AssertionError("Expected server NotFound") }
        catch (_: ClientException.NotFound) { }
        println("Core contract: $platform home/library pagination/credit pagination/search/artwork/local user/NotFound passed")
    } finally { client.cancelAll(); client.close() }
}

private fun ComposeTestRule.query(text: String) { onNodeWithTag("search-query").performScrollTo().performTextReplacement(text) }
private var activeRule: ComposeTestRule? = null
private var captureEnabled = false
private fun SemanticsNodeInteraction.performClick(): SemanticsNodeInteraction {
    performSemanticsAction(SemanticsActions.OnClick) { it() }
    activeRule?.awaitData()
    return this
}
private fun ComposeTestRule.awaitData() {
    waitForIdle()
    waitUntil(15_000) {
        onAllNodes(hasText("Loading", substring = true) or hasText("Saving preferences…") or hasText("Reading catalog…")).fetchSemanticsNodes().isEmpty()
    }
}
private fun SemanticsNodeInteraction.scrollOnMobile(tv: Boolean) = if (tv) assertIsDisplayed() else performScrollTo()

private fun ComposeTestRule.assertTvSidebarRemote(activity: Activity) {
    val instrumentation = InstrumentationRegistry.getInstrumentation()
    fun key(code: Int) { instrumentation.sendKeyDownUpSync(code); awaitData() }
    fun select(label: String) {
        val matcher = hasContentDescription(label)
        onNode(if (label == "Home" || label == "Movies") matcher and hasAnyAncestor(hasTestTag("sidebar")) else matcher).assertIsFocused()
        key(KeyEvent.KEYCODE_DPAD_CENTER)
    }
    onNodeWithContentDescription("Browse libraries").assertIsFocused()
    key(KeyEvent.KEYCODE_DPAD_LEFT)
    onNodeWithContentDescription("Collapse sidebar").assertIsFocused()
    onNodeWithContentDescription("Home").assertIsSelected()
    val sidebar = onNodeWithTag("sidebar").fetchSemanticsNode().boundsInRoot
    val content = onNodeWithTag("content").fetchSemanticsNode().boundsInRoot
    val fraction = sidebar.width / (sidebar.width + content.width)
    assertTrue("TV expanded sidebar fraction $fraction", fraction in .18f.. .20f)
    println("TV sidebar geometry: sidebar=$sidebar content=$content density=${activity.resources.displayMetrics.density} fraction=$fraction")
    onNodeWithContentDescription("Refresh").assertIsDisplayed()
    onNodeWithContentDescription("Log out").assertDoesNotExist()
    listOf("Home", "Search titles", "All libraries", "Movies", "TV Shows", "Refresh").forEach { label ->
        val row = onNodeWithContentDescription(label).fetchSemanticsNode().boundsInRoot
        assertTrue("$label row height", kotlin.math.abs(row.height / activity.resources.displayMetrics.density - 48f) < 1f)
        assertTrue("$label is inside sidebar", row.top >= sidebar.top && row.bottom <= sidebar.bottom)
    }
    key(KeyEvent.KEYCODE_DPAD_RIGHT)
    onNodeWithContentDescription("Browse libraries").assertIsFocused()
    onNodeWithContentDescription("Expand sidebar").assertIsDisplayed()
    assertTrue(kotlin.math.abs(onNodeWithTag("sidebar").fetchSemanticsNode().boundsInRoot.width / activity.resources.displayMetrics.density - 72f) < 1f)
    key(KeyEvent.KEYCODE_DPAD_LEFT)
    repeat(4) { key(KeyEvent.KEYCODE_DPAD_DOWN) }
    onNodeWithContentDescription("Movies").assertIsFocused().assertIsNotSelected()
    onNodeWithContentDescription("Home").assertIsSelected()
    onNode(hasText("Home") and !hasClickAction()).assertIsDisplayed()
    select("Movies")
    onNodeWithContentDescription("Nosferatu, Movie, 63–94m · version dependent").assertIsFocused()
    onNodeWithContentDescription("Expand sidebar").assertIsDisplayed()
    key(KeyEvent.KEYCODE_DPAD_LEFT)
    onNodeWithContentDescription("TV Shows").assertIsFocused().assertIsNotSelected()
    onNode(hasContentDescription("Movies") and hasAnyAncestor(hasTestTag("sidebar"))).assertIsSelected()
    key(KeyEvent.KEYCODE_DPAD_RIGHT)
    onNodeWithContentDescription("Nosferatu, Movie, 63–94m · version dependent").assertIsFocused()
    key(KeyEvent.KEYCODE_DPAD_LEFT)
    repeat(4) { key(KeyEvent.KEYCODE_DPAD_UP) }
    select("Home")
    onNodeWithContentDescription("Browse libraries").assertIsFocused()
    key(KeyEvent.KEYCODE_DPAD_LEFT)
    onNodeWithContentDescription("Collapse sidebar").assertIsFocused()
    repeat(6) { key(KeyEvent.KEYCODE_DPAD_DOWN) }
    onNodeWithContentDescription("Subtitles").assert(SemanticsMatcher.expectValue(SemanticsProperties.StateDescription, "Off, Collapsed"))
    select("Subtitles")
    onNodeWithContentDescription("Off").assertIsFocused().assertIsSelected()
    key(KeyEvent.KEYCODE_DPAD_DOWN)
    select("Library tracks")
    onNodeWithContentDescription("Subtitles").assertIsFocused().assert(SemanticsMatcher.expectValue(SemanticsProperties.StateDescription, "Library tracks, Collapsed"))
    select("Subtitles")
    key(KeyEvent.KEYCODE_DPAD_DOWN)
    select("Transcription")
    select("Subtitles")
    repeat(2) { key(KeyEvent.KEYCODE_DPAD_UP) }
    select("Off")
    key(KeyEvent.KEYCODE_DPAD_DOWN)
    select("Refresh")
    onNodeWithText("Catalog refreshed.").assertIsDisplayed()
    onNodeWithContentDescription("Log out").assertDoesNotExist()
    onNodeWithText("Welcome").assertDoesNotExist()
    onNodeWithContentDescription("Continue").assertDoesNotExist()
    repeat(6) { key(KeyEvent.KEYCODE_DPAD_UP) }
    select("Home")
    key(KeyEvent.KEYCODE_DPAD_RIGHT)
    onNodeWithContentDescription("Browse libraries").assertIsFocused()
}

private fun ComposeTestRule.awaitResult(text: String) {
    try { waitUntil(15_000) { onAllNodesWithText(text).fetchSemanticsNodes().isNotEmpty() } }
    catch (failure: androidx.compose.ui.test.ComposeTimeoutException) { throw AssertionError("Expected $text\n${onRoot().printToString()}", failure) }
}

private fun ComposeTestRule.awaitDetails(title: String) {
    val responseTitle = hasText(title, substring = false) and hasAnyAncestor(hasTestTag("title-details"))
    waitUntil(15_000) { onAllNodes(responseTitle).fetchSemanticsNodes().size == 1 }
    onNodeWithTag("title-details").assertIsDisplayed()
    onNode(responseTitle).assertIsDisplayed()
}

private fun ComposeTestRule.dismissKeyboard(activity: Activity, nativeFocus: Boolean = false) {
    val instrumentation = InstrumentationRegistry.getInstrumentation()
    instrumentation.setInTouchMode(false)
    if (nativeFocus) {
        onNodeWithTag("search-query").assertIsFocused()
        runOnIdle { activity.window.decorView.windowInsetsController!!.hide(WindowInsets.Type.ime()) }
        waitUntil(5_000) { !activity.window.decorView.rootWindowInsets.isVisible(WindowInsets.Type.ime()) }
        instrumentation.sendKeyDownUpSync(KeyEvent.KEYCODE_TAB)
        waitForIdle()
    } else onNodeWithContentDescription("Search settings").performScrollTo().performSemanticsAction(SemanticsActions.RequestFocus) { it() }
    onNodeWithContentDescription("Search settings").assertIsFocused()
    onNodeWithTag("search-query").assertIsNotFocused()
    runOnIdle { activity.window.decorView.windowInsetsController!!.hide(WindowInsets.Type.ime()) }
    if (!activity.packageManager.hasSystemFeature("android.software.leanback")) {
        InstrumentationRegistry.getInstrumentation().setInTouchMode(true)
    }
    waitUntil(5_000) { !activity.window.decorView.rootWindowInsets.isVisible(WindowInsets.Type.ime()) }
}

private fun ComposeTestRule.captureShowcase(flow: String, assertStable: () -> Unit = {}) {
    val instrumentation = InstrumentationRegistry.getInstrumentation()
    awaitData()
    waitUntil(15_000) { onAllNodesWithTag("art-ready", useUnmergedTree = true).fetchSemanticsNodes().isNotEmpty() }
    waitForIdle()
    instrumentation.uiAutomation.waitForIdle(1_000, 5_000)
    waitForIdle()
    assertTrue(onNodeWithTag("content").fetchSemanticsNode().config[SemanticsProperties.VerticalScrollAxisRange].value() == 0f)
    val sidebarScroll = onNodeWithTag("sidebar").fetchSemanticsNode().config.getOrNull(SemanticsProperties.VerticalScrollAxisRange)
        ?: onNodeWithTag("sidebar-navigation").fetchSemanticsNode().config[SemanticsProperties.VerticalScrollAxisRange]
    assertTrue(sidebarScroll.value() <= 1f)
    assertStable()
    if (!captureEnabled) return
    val image = requireNotNull(instrumentation.uiAutomation.takeScreenshot())
    assertStable()
    val output = File(instrumentation.targetContext.filesDir, "screenshots/$flow.png")
    assertTrue(output.parentFile!!.isDirectory || output.parentFile!!.mkdirs())
    output.outputStream().use { assertTrue(image.compress(Bitmap.CompressFormat.PNG, 100, it)) }
    image.recycle()
    assertTrue(output.length() > 0)
}
