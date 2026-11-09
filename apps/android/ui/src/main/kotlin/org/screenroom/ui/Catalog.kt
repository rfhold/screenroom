package org.screenroom.ui

import androidx.compose.runtime.*
import kotlinx.coroutines.sync.Mutex
import kotlinx.coroutines.sync.withLock
import kotlinx.coroutines.currentCoroutineContext
import kotlinx.coroutines.ensureActive
import org.screenroom.core.*

internal typealias Art = Artwork
internal data class Person(val name: String, val role: String, val art: Art?)
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
internal fun sourceLabel(source: String) = if (source == "all") "All" else source.replaceFirstChar { it.uppercase() }.let { if (source == "tmdb") source.uppercase() else it }
internal fun validLink(url: String): Boolean = try { java.net.URI(url).let { it.scheme in setOf("http", "https") && !it.host.isNullOrBlank() && it.userInfo == null } } catch (_: Exception) { false }
internal data class ProviderPresentation(val label: String, val url: String?)
internal fun ProviderReference.presentation(rating: Rating?, score: String): ProviderPresentation = ProviderPresentation(
    if (rating?.provider?.equals(provider, ignoreCase = true) == true) "$provider · $score" else "$provider: $status",
    url.takeIf(::validLink))

internal class CoreStore(endpoint: String, profile: String) {
    val client = Client(ClientConfig(endpoint, profile, 3000u))
    var catalog by mutableStateOf(emptyList<Title>())
    var homeRows by mutableStateOf(emptyList<Pair<String, List<Title>>>())
    var preferences by mutableStateOf<Preferences?>(null)
    var capabilities by mutableStateOf<SearchCapabilities?>(null)
    var libraries by mutableStateOf(emptyList<MediaLibrary>())
    var catalogInfo by mutableStateOf<CatalogInfo?>(null)
    var saving by mutableStateOf(false)
    private val preferenceLock = Mutex()
    val readRefresh = ReadRefresh()
    fun refreshReads() { client.invalidateReads(); readRefresh.advance() }
    fun libraryId(selection: String) = libraries.resolveLibrary(selection)?.id ?: error("No library supplied")
    fun libraryTitle(selection: String) = libraries.resolveLibrary(selection)?.title.orEmpty()
    suspend fun bootstrap() {
        val prefs = coreRequest { client.getPreferences() }
        val caps = coreRequest { client.getSearchCapabilities() }
        val libraries = coreRequest { client.listLibraries() }
        val info = coreRequest { client.getCatalogInfo() }
        val records = mutableListOf<Title>()
        for (library in libraries) records += items { client.listLibraryItems(library.id, it) }
        val rows = libraries.firstOrNull()?.let { library -> coreRequest { client.getHome(library.id) }.map { it.title to it.items.map(Item::ui) } }.orEmpty()
        currentCoroutineContext().ensureActive()
        preferences = prefs; capabilities = caps; catalog = records.distinctBy { it.id }; homeRows = rows
        this.libraries = libraries; catalogInfo = info
    }
    suspend fun items(read: suspend (org.screenroom.core.Page) -> ItemPage): List<Title> {
        return collectPages { token ->
            val result = coreRequest { read(org.screenroom.core.Page(100u, token)) }
            result.items.map(Item::ui) to result.nextPageToken
        }
    }
    suspend fun detail(id: String): Pair<Title, List<Title>> {
        val item = coreRequest { client.getItem(id) }.ui()
        val credits = collectPages { token ->
            val result = coreRequest { client.listCredits(id, org.screenroom.core.Page(100u, token)) }
            result.credits to result.nextPageToken
        }
        val full = item.copy(cast = credits.filter { it.department.equals("cast", true) }.map { Person(it.name, it.role, it.artwork) },
            crew = credits.filterNot { it.department.equals("cast", true) }.map { Person(it.name, it.role, it.artwork) })
        val children = if (item.kind in listOf("Series", "Season")) items { client.listChildren(id, it) } else emptyList()
        currentCoroutineContext().ensureActive()
        catalog = (catalog.filterNot { it.id == id } + full + children).distinctBy { it.id }
        return full to children
    }
    suspend fun update(subtitle: String? = null, types: Set<String>? = null) = preferenceLock.withLock {
        saving = true
        try {
        val current = preferences ?: coreRequest { client.getPreferences() }
        val next = current.copy(subtitlePreference = subtitle ?: current.subtitlePreference,
            searchTypes = types?.let { SearchTypes("Movie" in it, "Series" in it, "Episode" in it) } ?: current.searchTypes)
        preferences = coreRequest { client.updatePreferences(next, if (subtitle != null) listOf("subtitle_preference") else listOf("search_types")) }
        } finally { saving = false }
    }
}
internal val LocalCore = staticCompositionLocalOf<CoreStore> { error("Core not provided") }
