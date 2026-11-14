package org.screenroom.ui

import androidx.compose.runtime.*
import androidx.compose.ui.graphics.asImageBitmap
import kotlinx.coroutines.sync.Mutex
import kotlinx.coroutines.sync.withLock
import kotlinx.coroutines.currentCoroutineContext
import kotlinx.coroutines.ensureActive
import org.screenroom.core.*

internal typealias Art = Artwork
internal data class Person(val name: String, val role: String, val art: Art?)
internal data class HomeSection(val title: String, val items: List<Title>, val error: String?)
internal typealias Rights = org.screenroom.core.Rights
internal data class Title(val record: Item, val cast: List<Person> = emptyList(), val crew: List<Person> = emptyList()) {
    val id get() = record.id
    val title get() = record.title
    val kind get() = record.kind.replaceFirstChar { it.uppercase() }
    val year get() = record.year
    val meta get() = record.subtitle
    val rights get() = record.titleRights
    val overview get() = record.overview
    val parent get() = record.parentId
    val progress get() = record.progressPercent?.toInt()
    val watched get() = record.watched == true
    val playbackAccessibility get() = listOfNotNull(if (watched) "Watched" else null, progress?.let { "Playback progress $it%" }).joinToString(", ")
    fun labels(seasonContext: Boolean = false) = cardLabels(kind, title, meta, year, seasonContext)
    fun accessibility(seasonContext: Boolean = false) = if (kind != "Episode") listOf(title, kind, meta, playbackAccessibility).filter(String::isNotEmpty).joinToString(", ")
        else labels(seasonContext).let { listOf(it.heading, it.subtitle, playbackAccessibility).filter(String::isNotEmpty).joinToString(", ") }
    val cardAccessibility get() = accessibility()
    val date get() = record.releaseDate
    val artwork get() = record.artwork
    val ink get() = presentationColor(record.colors.getOrNull(0), 0xff202020)
    val light get() = presentationColor(record.colors.getOrNull(1), 0xff555555)
}
internal data class Methods(val text: Boolean = true, val fuzzy: Boolean = true, val semantic: Boolean = true, val titleOnly: Boolean = false)
internal fun Item.ui() = Title(this)
internal fun presentationColor(value: String?, fallback: Long): Long = value?.removePrefix("#")?.takeIf { it.length == 6 }?.toLongOrNull(16)?.or(0xff000000) ?: fallback
internal fun List<MediaLibrary>.resolveLibrary(selection: String) = firstOrNull { it.id == selection } ?: firstOrNull()
internal fun SearchCapabilities.defaultMethods() = Methods("text" in methods, "fuzzy" in methods, "semantic" in methods, false)
internal fun SearchCapabilities.canUseTitleOnly() = "title_only" in methods && ("text" in methods || "fuzzy" in methods)
internal fun SearchCapabilities.toggleTitleOnly(current: Methods): Methods {
    if (current.titleOnly) return current.copy(titleOnly = false)
    if (!canUseTitleOnly()) return current
    val text = "text" in methods && (current.text || !current.fuzzy)
    val fuzzy = "fuzzy" in methods && (current.fuzzy || !text)
    return Methods(text, fuzzy, false, true)
}
internal fun SearchCapabilities.allowedTypes(types: SearchTypes?) = this.types.filter { when (it) { "Movie" -> types?.movie == true; "Series" -> types?.series == true; "Episode" -> types?.episode == true; else -> false } }.toSet()
internal fun SearchCapabilities.allowedSource(source: String) = sources.firstOrNull { it == source } ?: sources.firstOrNull().orEmpty()
internal fun sourceLabel(source: String) = if (source == "all") "All" else source.replaceFirstChar { it.uppercase() }
internal fun validLink(url: String): Boolean = try { java.net.URI(url).let { it.scheme in setOf("http", "https") && !it.host.isNullOrBlank() && it.userInfo == null } } catch (_: Exception) { false }
internal data class ProviderPresentation(val label: String, val url: String?)
internal fun ProviderReference.presentation(rating: Rating?, score: String): ProviderPresentation = ProviderPresentation(
    if (rating?.provider?.equals(provider, ignoreCase = true) == true) "$provider · $score" else "$provider: $status",
    url.takeIf(::validLink))

internal class CoreStore(endpoint: String, userId: String, private val settingsStore: SettingsStore) {
    private val config = ClientConfig(endpoint, userId, 3000u)
    val client = Client(config)
    fun profileClient() = Client(config)
    var pendingProfileSource by mutableStateOf<String?>(null)
    var catalog by mutableStateOf(emptyList<Title>())
    var homeRows by mutableStateOf(emptyList<HomeSection>())
    val artworkRequests = ArtworkRequests { url: String ->
        val bytes = coreRequest { client.loadArtwork(url) }
        kotlinx.coroutines.withContext(kotlinx.coroutines.Dispatchers.Default) {
            android.graphics.BitmapFactory.decodeByteArray(bytes, 0, bytes.size)?.asImageBitmap()
                ?: error("Artwork unavailable")
        }
    }
    var homeScope by mutableStateOf<SourceLibraryScope?>(null)
    val localSettings = settingsStore.session
    val preferences get() = localSettings.value
    var capabilities by mutableStateOf<SearchCapabilities?>(null)
    var libraries by mutableStateOf(emptyList<MediaLibrary>())
    var saving by mutableStateOf(false)
    private val preferenceLock = Mutex()
    val readRefresh = ReadRefresh()
    private val bootstrapRequests = RequestGeneration()
    private val detailRequests = RequestGeneration()
    fun refreshReads() { client.invalidateReads(); readRefresh.advance() }
    fun libraryId(selection: String) = selection.takeUnless { it == "all" }.orEmpty()
    fun requestScope(selection: String) = SourceLibraryScope(preferences.scope?.sourceId.orEmpty(), libraryId(selection))
    fun libraryTitle(selection: String) = libraries.resolveLibrary(selection)?.title.orEmpty()
    suspend fun bootstrap(scope: SourceLibraryScope) = readOperation {
        val request = bootstrapRequests.next()
        val caps = coreRequest { client.getSearchCapabilities() }.let { it.copy(types = it.types.map { type -> type.replaceFirstChar(Char::uppercase) }) }
        val libraries = collectPages { token -> coreRequest { client.listLibraries(scope.sourceId, org.screenroom.core.Page(100u, token)) }.let { it.libraries to it.nextPageToken } }
        val rows = coreRequest { client.getHome(scope) }.map { HomeSection(it.title, it.items.map(Item::ui), it.readError?.let { error -> progressError(error.code) }) }
        currentCoroutineContext().ensureActive()
        if (!bootstrapRequests.current(request)) throw kotlinx.coroutines.CancellationException("Superseded bootstrap")
        val source = caps.allowedSource(preferences.source)
        if (source.isNotEmpty() && source != preferences.source) update(source = source)
        currentCoroutineContext().ensureActive()
        if (!bootstrapRequests.current(request)) throw kotlinx.coroutines.CancellationException("Superseded bootstrap")
        capabilities = caps; catalog = (rows.flatMap { it.items } + catalog).distinctBy { it.id }; homeRows = rows
        homeScope = scope
        this.libraries = listOf(MediaLibrary("all", "All libraries")) + libraries
    }
    suspend fun detail(id: String, library: String): Pair<Title, List<Title>> = readOperation {
        val request = detailRequests.next()
        val scope = requestScope(library)
        val item = coreRequest { client.getItem(id, scope) }.ui()
        val ancestors = loadParentChain(item, { it.id }, { it.parent }) { coreRequest { client.getItem(it, scope) }.ui() }
        val credits = collectPages { token ->
            val result = coreRequest { client.listCredits(id, org.screenroom.core.Page(100u, token), scope) }
            result.credits to result.nextPageToken
        }
        val full = item.copy(cast = credits.filter { it.department.equals("cast", true) }.map { Person(it.name, it.role, it.artwork) },
            crew = credits.filterNot { it.department.equals("cast", true) }.map { Person(it.name, it.role, it.artwork) })
        val children = emptyList<Title>()
        currentCoroutineContext().ensureActive()
        if (!detailRequests.current(request)) throw kotlinx.coroutines.CancellationException("Superseded detail")
        catalog = (listOf(full) + ancestors + children + catalog).distinctBy { it.id }
        full to children
    }
    suspend fun update(types: Set<String>? = null, methods: Methods? = null, source: String? = null, library: String? = null) = preferenceLock.withLock {
        saving = true
        try {
        val current = preferences
        val next = current.copy(types = types?.let { SearchTypes("Movie" in it, "Series" in it, "Episode" in it) } ?: current.types,
            methods = methods?.let { SearchMethods(it.titleOnly, it.text, it.fuzzy, it.semantic) } ?: current.methods,
            source = source ?: current.source,
            scope = library?.let { requestScope(it) } ?: current.scope)
        localSettings.update(next)
        } finally { saving = false }
    }
    suspend fun retrySettings() = preferenceLock.withLock {
        saving = true
        try { localSettings.retry() } finally { saving = false }
    }
}
internal val LocalCore = staticCompositionLocalOf<CoreStore> { error("Core not provided") }
