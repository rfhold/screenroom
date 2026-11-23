@file:androidx.annotation.OptIn(androidx.media3.common.util.UnstableApi::class)

package org.screenroom.ui

import androidx.compose.foundation.Canvas
import androidx.compose.foundation.Image
import androidx.compose.ui.layout.ContentScale
import androidx.compose.ui.platform.LocalUriHandler
import androidx.compose.foundation.background
import androidx.compose.foundation.border
import androidx.compose.foundation.clickable
import androidx.compose.foundation.horizontalScroll
import androidx.compose.foundation.layout.*
import androidx.compose.foundation.rememberScrollState
import androidx.compose.foundation.shape.RoundedCornerShape
import androidx.compose.foundation.text.BasicText
import androidx.compose.foundation.text.BasicTextField
import androidx.compose.foundation.verticalScroll
import androidx.compose.foundation.gestures.BringIntoViewSpec
import androidx.compose.foundation.gestures.LocalBringIntoViewSpec
import androidx.compose.runtime.*
import androidx.compose.runtime.saveable.rememberSaveable
import androidx.compose.runtime.saveable.rememberSaveableStateHolder
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.draw.clip
import androidx.compose.ui.focus.onFocusChanged
import androidx.compose.ui.focus.FocusRequester
import androidx.compose.ui.focus.focusRequester
import androidx.compose.ui.focus.focusProperties
import androidx.compose.foundation.focusGroup
import androidx.compose.ui.input.key.*
import androidx.compose.ui.platform.LocalConfiguration
import androidx.compose.ui.text.style.TextOverflow
import androidx.compose.ui.platform.LocalInputModeManager
import androidx.compose.ui.input.InputMode
import androidx.compose.ui.geometry.Offset
import androidx.compose.ui.geometry.Size
import androidx.compose.ui.graphics.Brush
import androidx.compose.ui.graphics.Color
import androidx.compose.ui.graphics.drawscope.Stroke
import androidx.compose.ui.platform.testTag
import androidx.compose.ui.semantics.*
import androidx.compose.ui.text.TextStyle
import androidx.compose.ui.text.font.FontFamily
import androidx.compose.ui.text.font.FontWeight
import androidx.compose.ui.unit.Dp
import androidx.compose.ui.unit.dp
import androidx.compose.ui.unit.sp
import kotlinx.coroutines.delay
import kotlinx.coroutines.launch
import kotlinx.coroutines.CancellationException
import kotlinx.coroutines.Job
import kotlinx.coroutines.CoroutineScope
import kotlinx.coroutines.cancel
import kotlinx.coroutines.currentCoroutineContext
import kotlinx.coroutines.isActive
import kotlinx.coroutines.flow.first
import kotlinx.coroutines.withTimeoutOrNull
import org.screenroom.core.SearchRequest
import org.screenroom.core.SearchTypes
import org.screenroom.core.SearchMethods

private data class SearchState(val query: String = "", val methods: Methods = Methods(), val settings: Boolean = false, val source: String = "", val types: Set<String> = emptySet())
private data class Page(val screen: String = "home", val library: String = "", val id: String? = null, val search: SearchState = SearchState(), val entry: String = java.util.UUID.randomUUID().toString())
private val LocalTvFocusReturn = compositionLocalOf<((FocusRequester) -> Unit)?> { null }
private class FocusReturn(val target: String?, val record: (String) -> Unit)
private val LocalFocusReturn = compositionLocalOf<FocusReturn?> { null }
private class EntryRead(val entry: String, val epoch: Int, val pages: EntryPages<Title>)
private val LocalEntryRead = staticCompositionLocalOf<EntryRead> { error("Missing navigation read owner") }

@Composable
private fun returnableTvFocus(identity: String? = null, enabled: Boolean = true): Modifier {
    val record = LocalTvFocusReturn.current ?: return Modifier
    val restoration = LocalFocusReturn.current
    val requester = remember { FocusRequester() }
    LaunchedEffect(identity, enabled) { if (enabled && identity != null && restoration?.target == identity) requester.requestFocus() }
    return Modifier.focusRequester(requester).onFocusChanged {
        if (it.isFocused) { record(requester); if (identity != null) restoration?.record?.invoke(identity) }
    }
}

private object NearestVisibleFocus : BringIntoViewSpec {
    override fun calculateScrollDistance(offset: Float, size: Float, containerSize: Float): Float = when {
        offset < 0 -> offset
        offset + size > containerSize -> offset + size - containerSize
        else -> 0f
    }
}

@OptIn(androidx.compose.foundation.ExperimentalFoundationApi::class)
@Composable
fun HomeScreen(television: Boolean = false, endpoint: String = "http://127.0.0.1:50051", userId: String = "", settingsNamespace: String = "default", debug: Boolean = false, settings: SettingsStore? = null, registerBack: (() -> Boolean) -> Unit = {}) {
    val configurationError = remember(endpoint, debug) { endpointError(endpoint, debug) }
    if (configurationError != null) {
        Column(Modifier.fillMaxSize().background(LocalScreenroomColors.current.background).padding(24.dp), verticalArrangement = Arrangement.spacedBy(16.dp)) {
            Heading("API configuration error")
            Label(configurationError)
        }
        return
    }
    val context = androidx.compose.ui.platform.LocalContext.current
    val settingsStore = remember(context, settingsNamespace, settings) { settings ?: SettingsStore(context, settingsNamespace) }
    val store = remember(endpoint, userId, settingsStore) { CoreStore(endpoint, userId, settingsStore) }
    DisposableEffect(store) { onDispose { store.artworkRequests.close(); store.client.cancelAll(); store.client.close() } }
    key(store) { CompositionLocalProvider(LocalCore provides store) { CoreHomeScreen(television, registerBack) } }
}

@OptIn(androidx.compose.foundation.ExperimentalFoundationApi::class)
@Composable
private fun CoreHomeScreen(television: Boolean, registerBack: (() -> Boolean) -> Unit) {
    val store = LocalCore.current
    val catalog = store.catalog
    val scope = rememberCoroutineScope()
    val context = androidx.compose.ui.platform.LocalContext.current
    var playback by remember { mutableStateOf<NativePlayback?>(null) }
    val recovery = remember { ProfileRecovery<NativePlayback>() }
    var playbackOwner by remember { mutableLongStateOf(recovery.capture()) }
    val presentation = remember { PlaybackPresentation() }
    var playbackTitle by remember { mutableStateOf("") }
    val views = rememberSaveableStateHolder()
    val focusedControls = remember { mutableMapOf<String, String>() }
    val scrollPositions = remember { mutableMapOf<String, Int>() }
    val entryPages = remember { EntryPages<Title>(id = { it.id }) }
    val readEpoch by store.readRefresh.generation.collectAsState()
    DisposableEffect(store) { onDispose { playback?.stop("client_changed") } }
    fun play(item: Title, library: String) {
        if (playback != null || recovery.visible?.cleanupOpen == true) return
        playbackOwner = recovery.capture()
        presentation.fullscreen = false
        playbackTitle = item.title
        playback = NativePlayback(context, store.profileClient(), item.id, store.requestScope(library))
    }
    var loading by remember { mutableStateOf(true) }
    var error by remember { mutableStateOf<String?>(null) }
    var reload by remember { mutableIntStateOf(0) }
    var status by rememberSaveable { mutableStateOf("") }
    var page by remember { mutableStateOf(Page(library = store.preferences.scope?.libraryId.orEmpty())) }
    var homeScope by remember { mutableStateOf(store.requestScope(page.library)) }
    LaunchedEffect(reload, homeScope) {
        loading = store.catalog.isEmpty() || store.homeScope != homeScope; error = null
        try { store.bootstrap(homeScope); if (reload > 0) status = "Catalog refreshed." } catch (e: CancellationException) { throw e } catch (_: Exception) { error = "Unable to read catalog. Retry to read server state."; status = "Catalog read failed." } finally { loading = false }
    }
    val history = remember { NavigationHistory<Page>() }
    var expanded by rememberSaveable { mutableStateOf(false) }
    val preferredTypes = store.capabilities?.allowedTypes(store.preferences.types).orEmpty()
    val refresh: () -> Unit = { store.refreshReads(); reload++; status = "Reading catalog…" }
    var detail by remember { mutableStateOf<Pair<Title, List<Title>>?>(null) }
    var detailLoading by remember { mutableStateOf(false) }
    var detailError by remember { mutableStateOf<String?>(null) }
    val detailRequests = remember { RequestGeneration() }
    LaunchedEffect(page.screen, page.library, page.id, reload) {
        val request = detailRequests.next()
        detail = null; detailError = null
        if (page.screen == "detail" && page.id != null) {
            detailLoading = true
            try { val response = store.detail(page.id!!, page.library); if (detailRequests.current(request)) detail = response }
            catch (e: CancellationException) { throw e }
            catch (e: Exception) { if (detailRequests.current(request)) detailError = e.message ?: "Details failed" }
            finally { if (detailRequests.current(request)) detailLoading = false }
        }
    }
    val contentFocus = remember { FocusRequester() }
    val sidebarFocus = remember { FocusRequester() }
    var lastContentFocus by remember(page.screen, page.library, page.id) { mutableStateOf<FocusRequester?>(null) }
    fun persist(change: suspend () -> Unit) { scope.launch { try { change() } catch (e: CancellationException) { throw e } catch (e: Exception) { status = "Settings save failed: ${e.message}" } } }
    fun changePage(next: Page) {
        page = next
        persist { store.update(library = next.library) }
        if (next.screen == "home") homeScope = store.requestScope(next.library)
    }
    fun forget(entry: Page) { views.removeState(entry.entry); focusedControls.remove(entry.entry); scrollPositions.remove(entry.entry); entryPages.remove(entry.entry) }
    fun navigate(next: Page) { history.push(page)?.let(::forget); changePage(next) }
    fun returnToPrevious() { val departed = page; changePage(history.pop()); forget(departed) }
    val back: () -> Boolean = {
        when { playback != null && presentation.exitFullscreen() -> true
            playback != null -> { playback?.let { it.stop("back"); recovery.retain(playbackOwner, it) }; playback = null; store.refreshReads(); reload++; true }
            page.search.settings && page.screen == "search" -> { page = page.copy(search = page.search.copy(settings = false)); true }
            expanded -> { expanded = false; true }
            history.canGoBack -> { returnToPrevious(); true }
            page.screen != "home" -> { forget(page); changePage(Page()); true }
            else -> false }
    }
    SideEffect { registerBack(back) }
    if (playback != null) {
        val currentPlayback = playback!!
        val currentOwner = playbackOwner
        PlaybackScreen(currentPlayback, playbackTitle, television, presentation) {
            if (recovery.current(currentOwner) && playback === currentPlayback) {
                recovery.retain(currentOwner, currentPlayback)
                playback = null
                store.refreshReads(); reload++
            }
        }
        return
    }
    val menu: @Composable () -> Unit = {
        val navigateMenu: (String) -> Unit = { target ->
            expanded = false
            when (target) {
                "Home" -> { history.clear().forEach(::forget); forget(page); changePage(Page()); status = "" }
                "Search titles" -> navigate(Page("search", page.library, page.id, SearchState(types = preferredTypes, methods = store.preferences.methods?.let { Methods(it.text, it.fuzzy, it.semantic, it.titleOnly) } ?: Methods(), source = store.preferences.source)))
                "Jellyfin profile" -> navigate(Page("profile", page.library))
                else -> navigate(Page("browse", target))
            }
        }
        if (television) TvSidebar(expanded, status, sidebarFocus,
            if (page.screen == "home") "Home" else if (page.screen == "search") "Search titles" else if (page.screen == "profile") "Jellyfin profile" else store.libraries.resolveLibrary(page.library)?.id.orEmpty(),
            { expanded = !expanded }, { expanded = true }, { expanded = false; (lastContentFocus ?: contentFocus).requestFocus() },
            refresh, navigateMenu)
        else Sidebar(false, expanded, status, { expanded = !expanded }, refresh, navigateMenu)
    }
    val content: @Composable () -> Unit = {
        CompositionLocalProvider(LocalEntryRead provides EntryRead(page.entry, readEpoch, entryPages)) {
        views.SaveableStateProvider(page.entry) { key(page.screen, page.library, page.id) {
            val entry = page.entry
            val scroll = rememberScrollState()
            val restore = remember(entry) { scrollPositions[entry] ?: scroll.value }
            DisposableEffect(entry, scroll) {
                onDispose { if (page.entry == entry || history.any { it.entry == entry }) scrollPositions[entry] = scroll.value }
            }
            LaunchedEffect(entry, scroll) {
                if (restore > 0) {
                    withTimeoutOrNull(1500) { snapshotFlow { scroll.maxValue }.first { it != Int.MAX_VALUE && it >= restore } }
                    scroll.scrollTo(restore.coerceAtMost(scroll.maxValue.coerceAtLeast(0)))
                }
            }
            Column(Modifier.fillMaxSize().testTag("content").verticalScroll(scroll).padding(if (television) 28.dp else 20.dp), verticalArrangement = Arrangement.spacedBy(22.dp)) {
            recovery.visible?.let { stopped ->
                val owner = recovery.capture()
                if (stopped.stopOutcome != StopOutcome.CONFIRMED) {
                    Label(stopped.message)
                    if (stopped.cleanupOpen) Label("Resolve this session before starting another. Recovery is available for up to one minute.")
                    StopRecoveryActions(stopped) { recovery.owns(owner, stopped) }
                }
            }
            if (page.screen != "home" && page.screen != "profile") Breadcrumbs(page,
                if (page.screen == "detail" && history.lastOrNull()?.screen == "search") ({ returnToPrevious() }) else null) { navigate(it) }
            if (loading) Label("Loading catalog…", Modifier.testTag("data-loading"))
            else if (error != null && (store.homeRows.isEmpty() || store.homeScope != homeScope)) { PageReadError(error!!, "Retry") { reload++ } }
            else when (page.screen) {
                "profile" -> JellyfinSettings(onReplacing = { recovery.replaceProfile() }, onChanged = { refresh() })
                "home" -> {
                    if (error != null) PageReadError(error!!, "Retry catalog") { reload++ }
                    val entryFocus = remember { FocusRequester() }
                    val returning = LocalFocusReturn.current?.target != null
                    LaunchedEffect(Unit) { if (television && !returning) entryFocus.requestFocus() }
                    Row(Modifier.fillMaxWidth(), horizontalArrangement = Arrangement.SpaceBetween, verticalAlignment = Alignment.CenterVertically) {
                        Heading("Home", modifier = Modifier.testTag("home-scope").semantics { stateDescription = store.homeScope?.let { "${it.sourceId}|${it.libraryId}" }.orEmpty() })
                        Action("Browse libraries", Modifier.focusRequester(entryFocus).then(if (television) Modifier.focusProperties { left = sidebarFocus } else Modifier)) { navigate(Page("browse")) }
                    }
                    store.homeRows.forEach { (name, items, rowError) ->
                        Column(verticalArrangement = Arrangement.spacedBy(14.dp)) {
                            Heading(name, small = true)
                            if (rowError != null) PageReadError(rowError, "Retry $name") { refresh() }
                            Row(Modifier.fillMaxWidth().horizontalScroll(rememberScrollState()), horizontalArrangement = Arrangement.spacedBy(16.dp)) {
                                items.forEach { item -> PosterCard(item, if (television) 220.dp else 144.dp, focusKey = "$name:${item.id}") { navigate(Page("detail", page.library, item.id)) } }
                            }
                        }
                    }
                }
                "search" -> {
                    Heading("Search")
                    SearchPanel(page.library, page.search.query, { page = page.copy(search = page.search.copy(query = it)) }, page.search.types, { types -> page = page.copy(search = page.search.copy(types = types)); scope.launch { try { store.update(types = types) } catch (e: CancellationException) { throw e } catch (e: Exception) { status = "Preferences update failed: ${e.message}" } } },
                        page.search.methods, { value -> page = page.copy(search = page.search.copy(methods = value)); persist { store.update(methods = value) } }, page.search.settings,
                        { page = page.copy(search = page.search.copy(settings = !page.search.settings)) }, page.search.source, { value -> page = page.copy(search = page.search.copy(source = value)); persist { store.update(source = value) } }, television) { navigate(Page("detail", page.library, it.id)) }
                }
                "browse", "detail" -> {
                    val item = detail?.first
                    when {
                        detailLoading && page.screen == "detail" -> Label("Loading details…")
                        detailError != null && page.screen == "detail" -> { Label("Unable to load details: $detailError"); Action("Retry details") { reload++ } }
                        item == null -> { Heading(store.libraryTitle(page.library)); LibraryGrid(page.library, television, reload) { navigate(Page("detail", page.library, it.id)) } }
                        item.kind == "Series" || item.kind == "Season" -> {
                            Detail(item, television)
                            ChildrenGrid(item, page.library, television, reload) { navigate(Page("detail", page.library, it.id)) }
                        }
                        else -> Detail(item, television) { play(item, page.library) }
                    }
                }
            }
            if (store.saving) Label("Saving preferences…")
            store.localSettings.warning?.let { warning ->
                PageReadError(warning, "Retry saving settings") { scope.launch { store.retrySettings() } }
            }
            Spacer(Modifier.height(12.dp))
        } } } }
    }
    val base = Modifier.fillMaxSize().background(LocalScreenroomColors.current.background).safeDrawingPadding()
    CompositionLocalProvider(LocalBringIntoViewSpec provides NearestVisibleFocus) {
        if (television) Row(base) { menu(); Box(Modifier.weight(1f).focusRequester(contentFocus).onFocusChanged { if (it.hasFocus) expanded = false }.focusGroup()) { CompositionLocalProvider(LocalTvFocusReturn provides { lastContentFocus = it }, LocalFocusReturn provides FocusReturn(focusedControls[page.entry]) { focusedControls[page.entry] = it }) { content() } } }
        else Column(base.imePadding()) { menu(); Box(Modifier.weight(1f)) { content() } }
    }
}

@Composable
private fun TvSidebar(expanded: Boolean, status: String, entryFocus: FocusRequester, destination: String,
    toggle: () -> Unit, enter: () -> Unit, leave: () -> Unit,
    refresh: () -> Unit, navigate: (String) -> Unit) {
    val width = (LocalConfiguration.current.screenWidthDp * .19f).dp.coerceIn(180.dp, 260.dp)
    val colors = LocalScreenroomColors.current
    val scroll = rememberScrollState()
    LaunchedEffect(expanded) { scroll.scrollTo(0) }
    Column(Modifier.width(if (expanded) width else 72.dp).fillMaxHeight().testTag("sidebar")
        .background(colors.surface).onFocusChanged { if (it.hasFocus) enter() }
        .onPreviewKeyEvent { event ->
            if (event.type == KeyEventType.KeyDown && event.key == Key.DirectionRight) { leave(); true }
            else false
        }.focusGroup().padding(horizontal = 8.dp, vertical = 6.dp)) {
        TvSidebarRow(if (expanded) "Collapse sidebar" else "Expand sidebar", "☰", expanded, modifier = Modifier.focusRequester(entryFocus)) { if (expanded) leave() else toggle() }
        Column(Modifier.weight(1f).testTag("sidebar-navigation").verticalScroll(scroll)) {
            val links = listOf(Triple("Home", "Home", "⌂"), Triple("Search titles", "Search titles", "⌕"), Triple("Jellyfin profile", "Jellyfin profile", "≡")) + LocalCore.current.libraries.map { Triple(it.id, it.title, "▤") }
            links.forEachIndexed { index, (id, name, icon) ->
                if (expanded && index == 3) Label("Libraries", Modifier.padding(start = 12.dp, top = 6.dp, bottom = 2.dp), muted = true, size = 12)
                TvSidebarRow(name, icon, expanded, selected = destination == id, click = { navigate(id) })
            }
        }
        if (expanded) {
            TvSidebarRow("Refresh", "↻", true, click = refresh)
            if (status.isNotEmpty()) Label(status, Modifier.padding(start = 12.dp).semantics { liveRegion = LiveRegionMode.Polite }, muted = true, size = 12)
        }
    }
}

@Composable
private fun TvSidebarRow(label: String, icon: String, expanded: Boolean, modifier: Modifier = Modifier,
    selected: Boolean = false, value: String? = null, disclosure: String? = null, click: () -> Unit) {
    var focused by remember { mutableStateOf(false) }
    val colors = LocalScreenroomColors.current
    Row(modifier.fillMaxWidth().height(48.dp).onFocusChanged { focused = it.isFocused }
        .clip(RoundedCornerShape(6.dp)).background(when { focused -> Color(0xff303030); selected -> Color(0xff252525); else -> Color.Transparent })
        .border(if (focused) 2.dp else 1.dp, if (focused) colors.accent else Color.Transparent, RoundedCornerShape(6.dp))
        .semantics { contentDescription = label; this.selected = selected; if (selected) stateDescription = "Selected" }
        .clickable(role = Role.Button, onClick = click).padding(horizontal = 10.dp), verticalAlignment = Alignment.CenterVertically) {
        Box(Modifier.width(24.dp), contentAlignment = Alignment.Center) { Label(icon, size = 20) }
        if (expanded) {
            Spacer(Modifier.width(8.dp))
            Column(Modifier.weight(1f)) {
                BasicText(if (label == "Search titles") "Search" else label, style = BodyStyle.copy(color = colors.text, fontSize = 14.sp, fontWeight = if (selected) FontWeight.SemiBold else FontWeight.Normal), maxLines = 1, overflow = TextOverflow.Ellipsis)
                if (value != null) BasicText(value, style = BodyStyle.copy(color = colors.secondary, fontSize = 12.sp), maxLines = 1, overflow = TextOverflow.Ellipsis)
            }
            if (disclosure != null) Label(disclosure)
            else if (selected) Box(Modifier.width(3.dp).height(18.dp).background(colors.accent))
        }
    }
}

@Composable
internal fun Label(text: String, modifier: Modifier = Modifier, muted: Boolean = false, size: Int = 14) {
    val colors = LocalScreenroomColors.current
    BasicText(text, modifier, style = BodyStyle.copy(color = if (muted) colors.secondary else colors.text, fontSize = size.sp, lineHeight = (size + 6).sp))
}

@Composable
internal fun Heading(text: String, small: Boolean = false, modifier: Modifier = Modifier) {
    BasicText(text, modifier.semantics { heading() }, style = BodyStyle.copy(color = LocalScreenroomColors.current.text, fontSize = if (small) 21.sp else 28.sp, lineHeight = if (small) 28.sp else 36.sp, fontWeight = FontWeight.SemiBold))
}

@Composable
internal fun Action(label: String, modifier: Modifier = Modifier, selected: Boolean = false, enabled: Boolean = true, visible: String = label, quiet: Boolean = false, click: () -> Unit) {
    var focus by remember { mutableStateOf(false) }
    val colors = LocalScreenroomColors.current
    val keyboard = LocalInputModeManager.current.inputMode == InputMode.Keyboard
    val showFocus = focus && keyboard
    Box(modifier.then(returnableTvFocus("action:$label", enabled)).onFocusChanged { focus = it.isFocused }.clip(RoundedCornerShape(8.dp))
        .background(if (selected || showFocus) Color(0xff303030) else Color.Transparent)
        .border(if (showFocus) 2.dp else 1.dp, if (showFocus) colors.accent else if (quiet) Color.Transparent else Color(0x22ffffff), RoundedCornerShape(8.dp))
        .semantics { contentDescription = label; if (selected) stateDescription = "Selected" }
        .clickable(enabled = enabled, role = Role.Button, onClick = click).sizeIn(minWidth = 48.dp, minHeight = 48.dp).padding(horizontal = 12.dp, vertical = 10.dp), contentAlignment = Alignment.CenterStart) {
        Label(visible, muted = !enabled)
    }
}

@Composable
private fun Sidebar(tv: Boolean, expanded: Boolean, status: String, toggle: () -> Unit, refresh: () -> Unit, navigate: (String) -> Unit) {
    val modifier = if (tv) Modifier.width(if (expanded) 264.dp else 72.dp).fillMaxHeight() else Modifier.fillMaxWidth().heightIn(max = if (expanded) 440.dp else 72.dp)
    val scroll = rememberScrollState()
    LaunchedEffect(expanded) { scroll.scrollTo(0) }
    Column(modifier.testTag("sidebar").background(LocalScreenroomColors.current.surface).verticalScroll(scroll).padding(12.dp), verticalArrangement = Arrangement.spacedBy(8.dp)) {
        val links = listOf(Triple("Home", "Home", "⌂"), Triple("Search titles", "Search titles", "⌕"), Triple("Jellyfin profile", "Jellyfin profile", "≡")) + LocalCore.current.libraries.map { Triple(it.id, it.title, "▤") }
        if (!tv && !expanded) Row(Modifier.fillMaxWidth(), horizontalArrangement = Arrangement.spacedBy(4.dp)) {
            Action("Expand sidebar", visible = "☰", quiet = true, click = toggle)
            Row(Modifier.weight(1f).testTag("sidebar-links").horizontalScroll(rememberScrollState()), horizontalArrangement = Arrangement.spacedBy(4.dp)) {
                links.forEach { (id, name, icon) -> Action(name, visible = icon, quiet = true) { navigate(id) } }
            }
        } else {
            Action(if (expanded) "Collapse sidebar" else "Expand sidebar", visible = "☰", quiet = true, click = toggle)
            links.forEachIndexed { i, (id, name, icon) ->
                if (expanded && i == 3) Label("Libraries", muted = true, size = 12)
                Action(name, Modifier.fillMaxWidth(), visible = if (expanded) "$icon   $name" else icon, quiet = true) { navigate(id) }
            }
            if (expanded) {
                Action("Refresh", Modifier.fillMaxWidth(), click = refresh)
                if (status.isNotEmpty()) Label(status, muted = true, size = 12)
            }
        }
    }
}

@Composable
private fun Breadcrumbs(page: Page, returnSearch: (() -> Unit)?, navigate: (Page) -> Unit) {
    val catalog = LocalCore.current.catalog
    Row(Modifier.fillMaxWidth().testTag("breadcrumbs").horizontalScroll(rememberScrollState()), verticalAlignment = Alignment.CenterVertically) {
        Action("Home", quiet = true) { navigate(Page()) }
        Label("›", muted = true)
        Action(LocalCore.current.libraryTitle(page.library), quiet = true) { navigate(Page("browse", page.library)) }
        val item = catalog.find { it.id == page.id }
        val parent = catalog.find { it.id == item?.parent }
        val grandparent = catalog.find { it.id == parent?.parent }
        (listOfNotNull(grandparent, parent) + if (page.screen == "search") listOfNotNull(item) else emptyList()).forEach { ancestor ->
            Label("›", muted = true)
            Action(ancestor.title, quiet = true) { navigate(Page("detail", page.library, ancestor.id)) }
        }
        if (returnSearch != null) { Label("›", muted = true); Action("Search results", quiet = true, click = returnSearch) }
        if (page.screen == "search" || item != null) {
            Label("›", muted = true)
            Label(if (page.screen == "search") "Search" else item!!.title, Modifier.padding(horizontal = 12.dp), muted = true)
        }
    }
}

@Composable
private fun Poster(item: Title, width: Dp, decorative: Boolean = false, seasonContext: Boolean = false) {
    val posterTitle = item.labels(seasonContext).heading
    val compactTitle = posterTitle.split(' ').any { it.length > 8 }
    Box(Modifier.width(width).height(width * 1.5f).clip(RoundedCornerShape(10.dp)).semantics {
        if (!decorative) contentDescription = listOf(item.title, "Poster", item.playbackAccessibility).filter { it.isNotEmpty() }.joinToString(", ")
    }) {
        val artwork = item.artwork
        if (artwork != null) ArtImage(artwork, Modifier.fillMaxSize()) else {
        Canvas(Modifier.fillMaxSize()) {
            val light = Color(item.light)
            drawRect(Brush.linearGradient(listOf(Color(item.ink), light), end = Offset(size.width * 1.5f, size.height * 1.6f)))
            drawCircle(light.copy(alpha = .6f), radius = size.width * .425f, center = Offset(size.width * .705f, size.height * .38f), style = Stroke(1.dp.toPx()))
            drawOval(Brush.linearGradient(listOf(Color.Black.copy(alpha = .13f), light.copy(alpha = .45f))), topLeft = Offset(-size.width * .35f, size.height * .56f), size = Size(size.width * 2, size.height))
        }
        BasicText(posterTitle,
            Modifier.fillMaxWidth().padding(horizontal = width * .12f).align(Alignment.Center),
            style = TextStyle(color = Color(0xfffff5e5), fontFamily = FontFamily.Serif, fontSize = (width.value * if (compactTitle) .14f else .175f).sp, lineHeight = (width.value * if (compactTitle) .15f else .185f).sp))
        BasicText("POSTER NOT INCLUDED", Modifier.align(Alignment.BottomStart).padding(start = width * .12f, bottom = 18.dp), style = TextStyle(color = Color(0x99ffffff), fontSize = 7.sp, letterSpacing = 1.sp))
        }
        item.progress?.let { progress -> Box(Modifier.align(Alignment.BottomStart).fillMaxWidth().height(4.dp).background(Color(0x55ffffff))) { Box(Modifier.fillMaxWidth(progress / 100f).fillMaxHeight().background(Color(0xffe9e9e9))) } }
        if (item.watched) {
            Canvas(Modifier.align(Alignment.TopEnd).padding(8.dp).size(26.dp)) {
                drawCircle(Color(0xee16251f))
                val ink = Color(0xffe3f4e9)
                drawLine(ink, Offset(size.width * .29f, size.height * .51f), Offset(size.width * .44f, size.height * .66f), 2.dp.toPx())
                drawLine(ink, Offset(size.width * .44f, size.height * .66f), Offset(size.width * .73f, size.height * .35f), 2.dp.toPx())
            }
        }
    }
}

private fun catalogItemTestTag(id: String): String = "screenroom_item_" +
    id.toByteArray(Charsets.UTF_8).joinToString("") { (it.toInt() and 0xff).toString(16).padStart(2, '0') }

@OptIn(androidx.compose.ui.ExperimentalComposeUiApi::class)
@Composable
private fun PosterCard(item: Title, width: Dp, modifier: Modifier = Modifier, seasonContext: Boolean = false, focusKey: String = item.id, open: () -> Unit) {
    var focused by remember { mutableStateOf(false) }
    val labels = item.labels(seasonContext)
    Column(modifier.width(width).then(returnableTvFocus("item:$focusKey")).onFocusChanged { focused = it.isFocused }.border(if (focused) 2.dp else 0.dp, if (focused) LocalScreenroomColors.current.accent else Color.Transparent, RoundedCornerShape(10.dp))
        .testTag(catalogItemTestTag(item.id))
        .semantics { testTagsAsResourceId = true; contentDescription = item.accessibility(seasonContext) }.clickable(role = Role.Button, onClick = open), verticalArrangement = Arrangement.spacedBy(7.dp)) {
        Box(Modifier.clearAndSetSemantics {}) { Poster(item, width, decorative = true, seasonContext = seasonContext) }
        Row(horizontalArrangement = Arrangement.spacedBy(6.dp), verticalAlignment = Alignment.CenterVertically, modifier = Modifier.clearAndSetSemantics {}) {
            if (labels.primary.isNotEmpty()) Label(labels.primary, Modifier.weight(1f, fill = false))
            labels.number?.let { Label(it, muted = true, size = 12) }
        }
        Label(labels.subtitle, Modifier.clearAndSetSemantics {}, muted = true, size = 12)
    }
}

@Composable
private fun PosterGrid(items: List<Title>, tv: Boolean, entryFocus: Boolean = true, season: Title? = null, open: (Title) -> Unit) {
    val requester = remember { FocusRequester() }
    val returning = LocalFocusReturn.current?.target != null
    LaunchedEffect(items.isNotEmpty()) { if (tv && entryFocus && !returning && items.isNotEmpty()) requester.requestFocus() }
    BoxWithConstraints(Modifier.fillMaxWidth()) {
        val columns = maxOf(1, ((maxWidth.value + 16) / (if (tv) 236 else 156)).toInt())
        val width = minOf(if (tv) 220.dp else 180.dp, (maxWidth - 16.dp * (columns - 1)) / columns)
        Column(verticalArrangement = Arrangement.spacedBy(24.dp)) {
            items.chunked(columns).forEach { row -> Row(horizontalArrangement = Arrangement.spacedBy(16.dp)) { row.forEach { item -> key(item.id) {
                val shortEpisode = episodeSeasonContext(item.kind, item.parent, season?.kind, season?.id)
                PosterCard(item, width, if (item == items.first()) Modifier.focusRequester(requester) else Modifier, shortEpisode) { open(item) }
            } } } }
        }
    }
}

@Composable
private fun Detail(item: Title, tv: Boolean, play: (() -> Unit)? = null) {
    val info: @Composable () -> Unit = {
        Column(verticalArrangement = Arrangement.spacedBy(12.dp), modifier = Modifier.widthIn(max = if (tv) 680.dp else 760.dp).testTag("title-details")) {
            Label("${item.kind} · ${item.year}", muted = true, size = 12); Heading(item.title)
            Label(item.meta + if (item.kind == "Episode") " · Aired ${item.date}" else "", muted = true)
            ProviderInfo(item, tv)
            if (play != null && item.kind in setOf("Movie", "Episode")) Action("Play", Modifier.testTag("play-title"), click = play)
            Label(item.overview)
        }
    }
    BoxWithConstraints(Modifier.widthIn(max = if (tv) 1280.dp else 1120.dp).fillMaxWidth().testTag("detail-header")) {
        if (maxWidth >= 640.dp) Row(horizontalArrangement = Arrangement.spacedBy(28.dp)) {
            Box(Modifier.testTag("detail-poster")) { Poster(item, if (tv) 140.dp else 200.dp) }
            Box(Modifier.weight(1f)) { info() }
        } else Column(verticalArrangement = Arrangement.spacedBy(20.dp)) {
            Box(Modifier.testTag("detail-poster")) { Poster(item, 160.dp) }; info()
        }
    }
    People("Cast", item.cast, tv)
    People("Crew", item.crew, tv)
}

@Composable
private fun SearchPanel(library: String, query: String, update: (String) -> Unit, types: Set<String>, setTypes: (Set<String>) -> Unit, methods: Methods, setMethods: (Methods) -> Unit, settings: Boolean, toggleSettings: () -> Unit, source: String, setSource: (String) -> Unit, tv: Boolean, open: (Title) -> Unit) {
    val store = LocalCore.current
    val capabilities = store.capabilities
    LaunchedEffect(capabilities?.sources, source) {
        val supported = capabilities?.allowedSource(source).orEmpty()
        if (supported.isNotEmpty() && supported != source) setSource(supported)
    }
    val libraryName = store.libraryTitle(library)
    val owner = LocalEntryRead.current
    val pages = owner.pages.get(owner.entry, listOf("search", query, library, types.toSet(), methods, source, store.requestScope(library), owner.epoch))
    val results = pages.records
    val searching = pages.busy
    val failure = pages.failure
    val scope = rememberCoroutineScope()
    val appendScope = remember(pages) { CoroutineScope(scope.coroutineContext + Job(scope.coroutineContext[Job])) }
    DisposableEffect(appendScope) { onDispose { appendScope.cancel() } }
    var searchStatus by pages::status
    val read: suspend (String) -> Pair<List<Title>, String> = { token ->
        val supportedSource = requireNotNull(store.capabilities) { "Search capabilities unavailable" }.allowedSource(source)
        check(supportedSource.isNotEmpty()) { "No search sources available" }
        val result = coreRequest { store.client.search(SearchRequest(query, store.libraryId(library), SearchTypes("Movie" in types, "Series" in types, "Episode" in types), SearchMethods(methods.titleOnly, methods.text, methods.fuzzy, methods.semantic), supportedSource, org.screenroom.core.Page(100u, token), store.requestScope(library).sourceId)) }
        val actual = result.effectiveMethods
        searchStatus = "Effective search: " + listOfNotNull("Text".takeIf { actual?.text == true }, "Fuzzy".takeIf { actual?.fuzzy == true }, "Semantic".takeIf { actual?.semantic == true }).joinToString(", ") + result.degradationReason.takeIf { it.isNotBlank() }?.let { " · Fallback: $it" }.orEmpty()
        result.items.map { it.ui() } to result.nextPageToken
    }
    LaunchedEffect(pages) {
            if (query.isBlank() || pages.loaded) return@LaunchedEffect
            delay(350)
            pages.load(reset = true, read = read)
    }
    Column(verticalArrangement = Arrangement.spacedBy(16.dp)) {
        var focused by remember { mutableStateOf(false) }
        BasicTextField(query, { update(it.take(capabilities?.maximumQueryLength?.coerceAtMost(Int.MAX_VALUE.toUInt())?.toInt() ?: 0)) }, Modifier.fillMaxWidth().heightIn(min = 52.dp).then(returnableTvFocus("search")).onFocusChanged { focused = it.isFocused }
            .border(if (focused) 2.dp else 1.dp, if (focused) LocalScreenroomColors.current.accent else Color(0x33ffffff), RoundedCornerShape(10.dp))
            .background(Color(0xff171717)).padding(14.dp).semantics { contentDescription = "Search $libraryName" }.testTag("search-query"), singleLine = true,
            textStyle = BodyStyle.copy(color = LocalScreenroomColors.current.text), cursorBrush = Brush.verticalGradient(listOf(Color.White, Color.White)),
            decorationBox = { field -> if (query.isEmpty()) Label("Search $libraryName", muted = true); field() })
        Row(horizontalArrangement = Arrangement.spacedBy(8.dp)) {
            Action("Search settings", selected = settings, visible = "Settings", click = toggleSettings)
            if (query.isNotEmpty()) Action("Clear query") { update("") }
        }
        if (settings) Column(Modifier.fillMaxWidth().background(LocalScreenroomColors.current.surface, RoundedCornerShape(12.dp)).padding(16.dp), verticalArrangement = Arrangement.spacedBy(10.dp)) {
            Heading("Catalog", small = true)
            capabilities?.sources?.forEach { id -> Choice(sourceLabel(id), source == id) { setSource(id) } }
            Label("Catalog membership, not playback availability.", muted = true, size = 12)
            Heading("Types", small = true)
            FlowRow(horizontalArrangement = Arrangement.spacedBy(8.dp), verticalArrangement = Arrangement.spacedBy(8.dp)) {
                capabilities?.types?.forEach { kind ->
                    val name = when (kind) { "Movie" -> "Movies"; "Series" -> "Shows"; "Episode" -> "Episodes"; else -> kind }
                    Choice(name, kind in types, !(kind in types && types.size == 1)) { setTypes(if (kind in types) types - kind else types + kind) }
                }
            }
            Heading("Search methods", small = true); Label("Keep at least one method selected.", muted = true, size = 12)
            if (capabilities?.methods?.contains("title_only") == true) Choice("Title only", methods.titleOnly, capabilities.canUseTitleOnly()) {
                setMethods(capabilities.toggleTitleOnly(methods))
            }
            val count = listOf(methods.text, methods.fuzzy, methods.semantic).count { it }
            capabilities?.methods?.forEach { method -> when (method) {
                "text" -> Choice(if (methods.titleOnly) "Text (titles only)" else "Text (titles and metadata)", methods.text, !(methods.text && count == 1)) { setMethods(methods.copy(text = !methods.text)) }
                "fuzzy" -> Choice("Fuzzy titles", methods.fuzzy, !(methods.fuzzy && count == 1)) { setMethods(methods.copy(fuzzy = !methods.fuzzy)) }
                "semantic" -> Choice("Semantic", methods.semantic, !methods.titleOnly && !(methods.semantic && count == 1)) { setMethods(methods.copy(semantic = !methods.semantic)) }
            } }
            capabilities?.semanticDescription?.takeIf { it.isNotBlank() }?.let { Label(it, muted = true, size = 12) }
        }
        Label(when { query.isBlank() -> "Enter a title to search $libraryName."; failure != null -> "Search failed: $failure"; searching || !pages.loaded -> "Searching titles…"; results.isEmpty() -> "No matches in $libraryName. Try another search."; else -> "${results.size} ${if (results.size == 1) "result" else "results"} in $libraryName" }, muted = true)
        if (query.isNotBlank() && pages.loaded) Label(searchStatus, Modifier.testTag("search-effective-methods"), muted = true, size = 12)
        if (query.isNotBlank()) {
            PosterGrid(results, tv, entryFocus = false, open = open)
            PageContinuation(pages) { appendScope.launch { pages.load(read = read) } }
        }
    }
}

@Composable
private fun Choice(name: String, checked: Boolean, enabled: Boolean = true, click: () -> Unit) {
    Action(name, selected = checked, enabled = enabled, visible = "${if (checked) "☑" else "☐"}  $name", modifier = Modifier.semantics { role = Role.Checkbox; toggleableState = if (checked) androidx.compose.ui.state.ToggleableState.On else androidx.compose.ui.state.ToggleableState.Off }, click = click)
}

@Composable
private fun JellyfinSettings(onReplacing: () -> Unit, onChanged: () -> Unit) {
    val store = LocalCore.current
    val client = remember(store) { store.profileClient() }
    val scope = rememberCoroutineScope()
    var sources by remember { mutableStateOf(emptyList<org.screenroom.core.JellyfinSource>()) }
    var profiles by remember { mutableStateOf(emptyList<org.screenroom.core.JellyfinProfile>()) }
    var source by remember { mutableStateOf(store.pendingProfileSource.orEmpty()) }
    var selected by remember { mutableStateOf("") }
    var link by remember { mutableStateOf<org.screenroom.core.JellyfinLink?>(null) }
    var known by remember { mutableStateOf(false) }
    var busy by remember { mutableStateOf(true) }
    var error by remember { mutableStateOf("") }
    var status by remember { mutableStateOf("") }
    val operations = remember { RequestGeneration() }
    DisposableEffect(client, scope) { onDispose { operations.next(); scope.cancel(); client.cancelAll(); client.close() } }
    suspend fun load(target: String = source) {
        val request = operations.next()
        val current = { operations.current(request) }
        busy = true; known = false; error = ""; status = ""; profiles = emptyList()
        try { readOperation {
            checkProfileOwner(current)
            client.invalidateReads()
            val available = collectPages { token -> coreRequest { client.listJellyfinSources(org.screenroom.core.Page(100u, token)) }.let { it.sources to it.nextPageToken } }.distinctBy { it.id }
            checkProfileOwner(current)
            sources = available
            source = sources.firstOrNull { it.id == target }?.id ?: sources.firstOrNull()?.id.orEmpty()
            if (source.isNotEmpty()) {
                val choices = collectPages { token -> coreRequest { client.listJellyfinProfiles(source, org.screenroom.core.Page(100u, token)) }.let { it.profiles to it.nextPageToken } }.distinctBy { it.id }
                val mapping = coreRequest { client.getJellyfinLink(source) }
                checkProfileOwner(current)
                profiles = choices; link = mapping
                selected = link?.jellyfinUserId ?: profiles.firstOrNull()?.id.orEmpty()
                known = true
                if (store.pendingProfileSource == source) { store.pendingProfileSource = null; onChanged() }
            }
        } } catch (e: CancellationException) {
            if (current() && currentCoroutineContext().isActive) error = "Unable to read Jellyfin profile settings. Retry to read the server state."
            else throw e
        }
        catch (_: Exception) { if (current()) error = "Unable to read Jellyfin profile settings. Retry to read the server state." }
        finally { if (current()) busy = false }
    }
    fun change(profile: String?) {
        if (busy || !known) return
        val request = operations.next()
        val current = { operations.current(request) }
        busy = true; known = false; error = ""; status = ""; store.pendingProfileSource = source
        // A dispatched mutation can replace personal identity even if its receipt is lost.
        onReplacing()
        scope.launch {
            try {
                val result = reconcileProfileChange(
                    mutate = {
                        if (profile == null) coreRequest { client.clearJellyfinLink(source) } == source
                        else coreRequest { client.setJellyfinLink(source, profile) }.let { it.sourceId == source && it.jellyfinUserId == profile }
                    },
                    invalidate = { client.invalidateReads() },
                    read = { coreRequest { client.getJellyfinLink(source) } },
                    matches = { if (profile == null) it == null else it != null && it.sourceId == source && it.jellyfinUserId == profile },
                    current = current)
                checkProfileOwner(current)
                link = result.link; known = true; store.pendingProfileSource = null
                selected = link?.jellyfinUserId ?: profiles.firstOrNull()?.id.orEmpty()
                if (result.confirmed) status = if (result.ambiguous) "Server state checked: profile change is saved." else "Profile change saved on the server."
                else error = "The requested change was not confirmed. Current server state is shown; choose a profile before trying again."
                onChanged()
            } catch (e: CancellationException) {
                if (current() && currentCoroutineContext().isActive) error = "Profile change could not be confirmed. Retry reads server state only; the change will not be repeated."
                else throw e
            }
            catch (_: Exception) { if (current()) error = "Profile change could not be confirmed. Retry reads server state only; the change will not be repeated." }
            finally { if (current()) busy = false }
        }
    }
    LaunchedEffect(store) { load() }
    Heading("Jellyfin profile")
    Label("Server-saved progress mapping for this local user selector. This is not sign-in.", muted = true)
    if (busy) Label("Loading Jellyfin profile settings…")
    if (error.isNotEmpty()) {
        Label(error, Modifier.semantics { liveRegion = LiveRegionMode.Polite })
        Action("Retry Jellyfin settings", enabled = !busy) { scope.launch { load() } }
    }
    if (status.isNotEmpty()) Label(status, Modifier.semantics { liveRegion = LiveRegionMode.Polite })
    if (sources.isNotEmpty()) {
        Heading("Jellyfin source", small = true)
        sources.forEach { entry -> Choice(entry.displayName, entry.id == source, !busy) { scope.launch { load(entry.id) } } }
        Label("Current profile: " + if (!known) "Not confirmed" else if (link == null) "Not linked" else profiles.firstOrNull { it.id == link?.jellyfinUserId }?.displayName ?: "Linked profile unavailable in choices")
        Heading("Profile", small = true)
        profiles.forEach { entry -> Choice(entry.displayName, entry.id == selected, !busy && known) { selected = entry.id } }
        if (known && profiles.isEmpty()) Label("No enabled profiles are available.")
        Action("Save Jellyfin profile", enabled = !busy && known && profiles.any { it.id == selected }) { change(selected) }
        Action("Unlink Jellyfin profile", enabled = !busy && known && link != null) { change(null) }
    } else if (!busy && error.isEmpty()) Label("No Jellyfin sources are available.")
}

@Composable
private fun ArtImage(art: Art, modifier: Modifier) {
    val store = LocalCore.current
    val bitmap by produceState<androidx.compose.ui.graphics.ImageBitmap?>(null, art.url) {
        value = null
        store.readRefresh.repeatOnRefresh {
            if (value == null) {
                try { value = store.artworkRequests.load(art.url) }
                catch (e: CancellationException) { throw e } catch (_: Exception) { value = null }
            }
        }
    }
    if (bitmap != null) Image(bitmap!!, art.label, modifier.clip(RoundedCornerShape(10.dp)).testTag("art-ready"), contentScale = if (art.kind == "logo") ContentScale.Fit else ContentScale.Crop)
    else Box(modifier.background(LocalScreenroomColors.current.surface).semantics { contentDescription = "Artwork unavailable: ${art.label}" }, contentAlignment = Alignment.Center) { Label("Artwork unavailable", muted = true, size = 12) }
}

@Composable
private fun Link(label: String, url: String) {
    if (!validLink(url)) return
    val handler = LocalUriHandler.current
    var unavailable by remember(url) { mutableStateOf(false) }
    Action(label) { try { handler.openUri(url) } catch (_: Exception) { unavailable = true } }
    if (unavailable) Label("No application is available to open this link.")
}

@Composable
private fun ProviderInfo(item: Title, tv: Boolean) {
    var info by rememberSaveable(item.id) { mutableStateOf(false) }
    val rating = ratingPresentation(item.record.rating, item.record.metadataRetrievedAt)
    val entryFocus = remember { FocusRequester() }
    val returning = LocalFocusReturn.current?.target != null
    LaunchedEffect(item.id) { if (tv && !returning) entryFocus.requestFocus() }
    val score = rating.score
    Row(Modifier.horizontalScroll(rememberScrollState()), horizontalArrangement = Arrangement.spacedBy(6.dp), verticalAlignment = Alignment.CenterVertically) {
        val handler = LocalUriHandler.current
        var linkError by remember(item.id) { mutableStateOf(false) }
        item.record.providerReferences.forEachIndexed { index, provider ->
            val hasRating = item.record.rating?.provider?.equals(provider.provider, ignoreCase = true) == true
            val presentation = provider.presentation(item.record.rating, score)
            val label = presentation.label
            val modifier = if (index == 0) Modifier.focusRequester(entryFocus) else Modifier
            if (presentation.url != null) Action(label, modifier, visible = if (hasRating) "★ $label" else "${provider.provider} · ${providerStatus(provider.status)}") { try { handler.openUri(presentation.url) } catch (_: Exception) { linkError = true } }
            else Label(if (hasRating) label else "${provider.provider} · ${providerStatus(provider.status)}", Modifier.semantics { contentDescription = label }, muted = true, size = 12)
        }
        item.record.rating?.let { value -> if (item.record.providerReferences.none { it.provider.equals(value.provider, true) }) Label("${value.provider} · $score") }
        Action("Rating and catalog details", if (item.record.providerReferences.firstOrNull()?.let { validLink(it.url) } != true) Modifier.focusRequester(entryFocus) else Modifier, visible = "ⓘ", selected = info, quiet = true) { info = !info }
        if (linkError) Label("No application is available to open this link.")
    }
    if (info) Column(verticalArrangement = Arrangement.spacedBy(6.dp)) {
        Label(rating.disclosure, muted = true, size = 12)
        item.record.providerReferences.forEach { Label("${it.provider} · ${it.status}", muted = true, size = 12) }
        Label("Catalog membership is not playability.", muted = true, size = 12)
        Label("Metadata snapshot · ${item.record.metadataRetrievedAt.ifBlank { "Not supplied" }}", muted = true, size = 12)
        item.record.runtime?.let { runtime -> Label(runtime.display, muted = true, size = 12) }
    }
}

@Composable
private fun People(title: String, people: List<Person>, tv: Boolean) {
    Column(Modifier.fillMaxWidth().testTag("people-$title"), verticalArrangement = Arrangement.spacedBy(14.dp)) {
        Heading(title, small = true)
        if (people.isEmpty()) Label("No ${title.lowercase()} credits listed in this snapshot.", muted = true)
        else BoxWithConstraints(Modifier.fillMaxWidth()) {
            val width = if (tv) 90.dp else if (maxWidth >= 640.dp) 140.dp else 128.dp
            FlowRow(horizontalArrangement = Arrangement.spacedBy(16.dp), verticalArrangement = Arrangement.spacedBy(20.dp)) {
                people.forEach { person -> Column(Modifier.width(width), verticalArrangement = Arrangement.spacedBy(6.dp)) {
                    if (person.art != null) ArtImage(person.art, Modifier.size(width))
                    else Box(Modifier.size(width).background(LocalScreenroomColors.current.surface, RoundedCornerShape(10.dp)).semantics { contentDescription = "No portrait included for ${person.name}" }, contentAlignment = Alignment.Center) { Label(person.name.split(' ').take(2).joinToString("") { it.take(1) }, size = 28) }
                    Label(person.name, Modifier.heightIn(min = 38.dp), size = 13)
                    Label(person.role, Modifier.heightIn(min = 36.dp), muted = true, size = 12)
                    if (person.art?.kind == "character still") Label("Character still", muted = true, size = 12)
                } }
            }
        }
    }
}

@Composable
private fun LibraryGrid(library: String, tv: Boolean, reload: Int, open: (Title) -> Unit) {
    val store = LocalCore.current
    ItemPages(listOf("library", store.requestScope(library)), reload, tv, read = { token ->
        coreRequest { store.client.listLibraryItems(store.requestScope(library), org.screenroom.core.Page(100u, token)) }.let { it.items.map { item -> item.ui() } to it.nextPageToken }
    }, open = open)
}

@Composable
private fun ChildrenGrid(item: Title, library: String, tv: Boolean, reload: Int, open: (Title) -> Unit) {
    val store = LocalCore.current
    ItemPages(listOf("children", item.id, store.requestScope(library)), reload, tv, entryFocus = false, season = item, read = { token ->
        coreRequest { store.client.listChildren(item.id, org.screenroom.core.Page(100u, token), store.requestScope(library)) }
            .let { it.items.map { child -> child.ui() } to it.nextPageToken }
    }, open = open)
}

@Composable
private fun ItemPages(identity: Any, reload: Int, tv: Boolean, entryFocus: Boolean = true, season: Title? = null,
    read: suspend (String) -> Pair<List<Title>, String>, open: (Title) -> Unit) {
    val store = LocalCore.current
    val owner = LocalEntryRead.current
    val state = owner.pages.get(owner.entry, listOf(identity, owner.epoch, reload))
    val scope = rememberCoroutineScope()
    val appendScope = remember(state) { CoroutineScope(scope.coroutineContext + Job(scope.coroutineContext[Job])) }
    DisposableEffect(appendScope) { onDispose { appendScope.cancel() } }
    LaunchedEffect(state) { if (!state.loaded) state.load(reset = true, read = read) }
    PosterGrid(state.records, tv, entryFocus = entryFocus, season = season, open = open)
    PageContinuation(state) { appendScope.launch { state.load(read = read) } }
}

@Composable
private fun PageContinuation(state: IncrementalPages<Title>, load: () -> Unit) {
    Label("${state.records.size} loaded", Modifier.semantics { liveRegion = LiveRegionMode.Polite }, muted = true)
    if (state.busy) Label("Loading items…")
    state.failure?.let { Label(it) }
    if (state.failure != null || state.token.isNotEmpty()) Action(if (state.failure == null) "Load more" else "Retry loading items", enabled = !state.busy, click = load)
}

@Composable
fun PageReadError(message: String, retryLabel: String, retry: () -> Unit) {
    Label(message); Action(retryLabel, click = retry)
}
