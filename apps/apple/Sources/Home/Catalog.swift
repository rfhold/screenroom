import Foundation
import Observation

struct MetadataRecord {
    let date: String
    let rating: Rating?
    let providers: [ProviderReference]
    let retrievedAt: String
    var score: String {
        guard let rating else { return "—" }
        return rating.votes == 0 ? "Unrated" : String(format: "%.1f/%g", rating.value, rating.scale)
    }
    var ratingDetail: String {
        guard let rating else { return "Rating snapshot not imported." }
        return "\(rating.provider) user rating: \(score) · \(rating.votes.map { "\($0) votes" } ?? "Vote count not available")."
    }
}

func externalURL(_ value: String) -> URL? {
    guard let url = URL(string: value), ["https", "http"].contains(url.scheme), url.host != nil else { return nil }
    return url
}

func choiceLabel(_ value: String) -> String {
    switch value {
    case "all": "All catalogs"
    case "tmdb": "TMDB"
    case "jellyfin": "Jellyfin"
    case "text": "Text"
    case "fuzzy": "Fuzzy"
    case "semantic": "Semantic"
    case "title_only": "Title only"
    case "Movie": "Movies"
    case "Series": "Shows"
    case "Episode": "Episodes"
    default: value
    }
}

struct CatalogItem: Identifiable, Hashable {
    let value: Item
    var id: String { value.id }
    var title: String { value.title }
    var kind: String { value.kind }
    var year: Int { Int(value.year) }
    var meta: String { value.subtitle.isEmpty ? value.runtime?.display ?? "" : value.subtitle }
    var overview: String { value.overview }
    var ink: String { value.colors.first ?? "303030" }
    var light: String { value.colors.last ?? "707070" }
    var parent: String? { value.parentId }
    var progress: Double? { value.progressPercent.map(Double.init) }
    var rights: String { [value.titleRights?.label, value.titleRights?.note].compactMap { $0 }.joined(separator: " · ") }
    var rightsSource: String { value.titleRights?.source ?? "" }
    var attribution: String? { value.titleRights?.attribution }
    var licenseURL: String? { value.titleRights?.licenseUrl.isEmpty == false ? value.titleRights?.licenseUrl : nil }
    var artwork: Artwork? { value.artwork }
    var record: MetadataRecord? {
        guard value.rating != nil || !value.providerReferences.isEmpty else { return nil }
        return MetadataRecord(date: value.releaseDate, rating: value.rating, providers: value.providerReferences, retrievedAt: value.metadataRetrievedAt)
    }
}

struct SearchOptions: Equatable {
    var source = "all"
    var types: Set<String> = []
    var methods: Set<String> = []
    var titleOnly = false
    var coreTypes: SearchTypes { SearchTypes(movie: types.contains("Movie"), series: types.contains("Series"), episode: types.contains("Episode")) }
    var coreMethods: SearchMethods { SearchMethods(titleOnly: titleOnly, text: methods.contains("text"), fuzzy: methods.contains("fuzzy"), semantic: methods.contains("semantic")) }
    mutating func reconcile(_ capabilities: SearchCapabilities, preferences: Preferences?) {
        types.formIntersection(capabilities.types)
        if types.isEmpty {
            if let saved = preferences?.searchTypes {
                types = Set(capabilities.types.filter { ($0 == "Movie" && saved.movie) || ($0 == "Series" && saved.series) || ($0 == "Episode" && saved.episode) })
            }
            if types.isEmpty, let first = capabilities.types.first { types.insert(first) }
        }
        let available = capabilities.methods.filter { $0 != "title_only" }
        methods.formIntersection(available)
        if methods.isEmpty { methods = Set(available) }
        if !capabilities.methods.contains("title_only") { titleOnly = false }
        if titleOnly { methods.remove("semantic") }
        if !capabilities.sources.contains(source) { source = capabilities.sources.first ?? "" }
    }
}

@MainActor @Observable
final class CatalogStore {
    let client: Client?
    let pageSize: UInt32
    var libraries: [MediaLibrary] = []
    var rows: [HomeRow] = []
    var items: [CatalogItem] = []
    var children: [CatalogItem] = []
    var credits: [Credit] = []
    var results: [CatalogItem] = []
    var detail: CatalogItem?
    var preferences: Preferences?
    var capabilities: SearchCapabilities?
    var catalogInfo: CatalogInfo?
    var error = ""
    var loading = false
    var loadedArtwork: Set<String> = []
    var failedArtwork: Set<String> = []
    var artworkRevision = 0
    var provenanceItems: [CatalogItem] = []
    var provenanceArtwork: [Artwork] = []
    var provenanceLoading = false
    var provenanceError = ""
    private var provenanceGeneration = 0
    private var generation = 0
    private var searchGeneration = 0
    private var preferenceGeneration = 0
    private var configurationError = ""

    private static func validateEndpoint(_ endpoint: String) throws {
        guard let url = URLComponents(string: endpoint), let host = url.host,
              url.user == nil, url.password == nil, url.query == nil, url.fragment == nil,
              url.path.isEmpty || url.path == "/" else {
            throw NSError(domain: "ScreenroomConfiguration", code: 1, userInfo: [NSLocalizedDescriptionKey: "Invalid API endpoint configuration."])
        }
        if url.scheme == "https" { return }
        #if DEBUG
        if url.scheme == "http", ["127.0.0.1", "localhost", "::1", "[::1]"].contains(host.lowercased()) { return }
        #endif
        throw NSError(domain: "ScreenroomConfiguration", code: 2, userInfo: [NSLocalizedDescriptionKey: "API endpoint requires HTTPS; debug builds allow HTTP only on loopback."])
    }

    init() {
        let env = ProcessInfo.processInfo.environment
        let args = ProcessInfo.processInfo.arguments
        pageSize = min(100, max(1, UInt32(env["SCREENROOM_API_PAGE_SIZE"] ?? "100") ?? 100))
        func config(_ name: String, _ fallback: String) -> String {
            if let index = args.firstIndex(of: "--\(name)"), args.indices.contains(index + 1) { return args[index + 1] }
            return env[name] ?? fallback
        }
        do {
            let endpoint = config("SCREENROOM_API_ENDPOINT", "http://127.0.0.1:50051")
            try Self.validateEndpoint(endpoint)
            client = try Client(config: ClientConfig(endpoint: endpoint, mockProfile: config("SCREENROOM_MOCK_PROFILE", "apple-\(UUID().uuidString)"), timeoutMs: 5000))
        } catch { client = nil; configurationError = error.localizedDescription; self.error = configurationError }
    }

    init(client: Client, pageSize: UInt32 = 100) {
        self.client = client
        self.pageSize = pageSize
    }

    func libraryTitle(_ id: String) -> String { libraries.first { $0.id == id }?.title ?? "Library" }

    func invalidate() {
        client?.invalidateReads()
        artworkRevision += 1
        loadedArtwork = []; failedArtwork = []
    }

    func load(screen: String, library: String, itemID: String?, forceBootstrap: Bool = false) async {
        generation += 1
        let current = generation
        detail = nil; credits = []; children = []
        loading = true; error = ""
        guard let client else { error = configurationError; loading = false; return }
        do {
            if libraries.isEmpty || forceBootstrap {
                let values = try await client.listLibraries()
                try Task.checkCancellation()
                let prefs = try await client.getPreferences()
                try Task.checkCancellation()
                let caps = try await client.getSearchCapabilities()
                try Task.checkCancellation()
                guard current == generation else { return }
                libraries = values; preferences = prefs; capabilities = caps
            }
            if let itemID {
                let item = try await client.getItem(id: itemID)
                try Task.checkCancellation()
                var people: [Credit] = []
                var token = ""
                repeat {
                    try Task.checkCancellation()
                    let page = try await client.listCredits(itemId: itemID, page: Page(size: pageSize, token: token))
                    try Task.checkCancellation()
                    people += page.credits; token = page.nextPageToken
                } while !token.isEmpty
                var descendants: [Item] = []
                token = ""
                repeat {
                    try Task.checkCancellation()
                    let page = try await client.listChildren(parentId: itemID, page: Page(size: pageSize, token: token))
                    try Task.checkCancellation()
                    descendants += page.items; token = page.nextPageToken
                } while !token.isEmpty
                try Task.checkCancellation()
                guard current == generation else { return }
                detail = CatalogItem(value: item); credits = people; children = descendants.map(CatalogItem.init)
            } else if screen == "Home" {
                let values = try await client.getHome(libraryId: library)
                try Task.checkCancellation()
                guard current == generation else { return }
                rows = values
            } else if screen == "Library" {
                var values: [Item] = []; var token = ""
                repeat {
                    try Task.checkCancellation()
                    let page = try await client.listLibraryItems(libraryId: library, page: Page(size: pageSize, token: token))
                    try Task.checkCancellation()
                    values += page.items; token = page.nextPageToken
                } while !token.isEmpty
                try Task.checkCancellation()
                guard current == generation else { return }
                items = values.map(CatalogItem.init)
            }
        } catch {
            if !Task.isCancelled && current == generation { self.error = "Unable to load: \(error.localizedDescription)" }
        }
        if current == generation { loading = false }
    }

    func loadProvenance() async {
        provenanceGeneration += 1
        let current = provenanceGeneration
        provenanceLoading = true; provenanceError = ""
        provenanceItems = []; provenanceArtwork = []
        defer { if current == provenanceGeneration { provenanceLoading = false } }
        guard let client else { provenanceError = configurationError; return }
        do {
            let info = try await client.getCatalogInfo()
            try Task.checkCancellation()
            var titles: [String: Item] = [:]
            var token = ""
            repeat {
                try Task.checkCancellation()
                let page = try await client.listLibraryItems(libraryId: "all", page: Page(size: pageSize, token: token))
                try Task.checkCancellation()
                for item in page.items { titles[item.id] = item }
                token = page.nextPageToken
            } while !token.isEmpty
            var pending = titles.keys.sorted()
            var visited: Set<String> = []
            while !pending.isEmpty {
                try Task.checkCancellation()
                let id = pending.removeFirst()
                guard visited.insert(id).inserted, titles[id] != nil else { continue }
                token = ""
                repeat {
                    try Task.checkCancellation()
                    let page = try await client.listCredits(itemId: id, page: Page(size: pageSize, token: token))
                    try Task.checkCancellation()
                    token = page.nextPageToken
                } while !token.isEmpty
                token = ""
                repeat {
                    try Task.checkCancellation()
                    let page = try await client.listChildren(parentId: id, page: Page(size: pageSize, token: token))
                    try Task.checkCancellation()
                    for child in page.items {
                        titles[child.id] = child
                        if !visited.contains(child.id) { pending.append(child.id) }
                    }
                    token = page.nextPageToken
                } while !token.isEmpty
            }
            guard current == provenanceGeneration else { return }
            provenanceItems = titles.values.sorted { $0.id < $1.id }.map(CatalogItem.init)
            catalogInfo = info
            provenanceArtwork = info.artwork
        } catch {
            if !Task.isCancelled && current == provenanceGeneration { provenanceError = "Unable to load credits: \(error.localizedDescription)" }
        }
    }

    func search(query: String, library: String, options: SearchOptions) async {
        searchGeneration += 1
        let current = searchGeneration
        results = []
        guard !query.trimmingCharacters(in: .whitespaces).isEmpty, let client else { return }
        do {
            try await Task.sleep(for: .milliseconds(350))
            var values: [Item] = []; var token = ""
            repeat {
                try Task.checkCancellation()
                let page = try await client.search(request: SearchRequest(query: query, libraryId: library, types: options.coreTypes, methods: options.coreMethods, source: options.source, page: Page(size: pageSize, token: token)))
                try Task.checkCancellation()
                values += page.items; token = page.nextPageToken
            } while !token.isEmpty
            try Task.checkCancellation()
            guard current == searchGeneration else { return }
            results = values.map(CatalogItem.init); error = ""
        } catch {
            if !Task.isCancelled && current == searchGeneration { self.error = "Search unavailable: \(error.localizedDescription)" }
        }
    }

    func save(types: SearchTypes? = nil, subtitle: String? = nil) async {
        guard let client, var value = preferences else { return }
        preferenceGeneration += 1
        let current = preferenceGeneration
        var mask: [String] = []
        if let types { value.searchTypes = types; mask.append("search_types") }
        if let subtitle { value.subtitlePreference = subtitle; mask.append("subtitle_preference") }
        do {
            let saved = try await client.updatePreferences(preferences: value, updateMask: mask)
            try Task.checkCancellation()
            if current == preferenceGeneration { preferences = saved }
        } catch { if !Task.isCancelled && current == preferenceGeneration { self.error = "Preferences not saved: \(error.localizedDescription)" } }
    }
}
