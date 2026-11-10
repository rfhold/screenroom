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

func searchType(_ value: String) -> String {
    switch value.lowercased() {
    case "movie": "Movie"
    case "series": "Series"
    case "episode": "Episode"
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
    mutating func reconcile(_ capabilities: SearchCapabilities, preferences: ClientSettings?) {
        let supportedTypes = capabilities.types.map(searchType)
        types.formIntersection(supportedTypes)
        if types.isEmpty {
            if let saved = preferences?.types {
                types = Set(supportedTypes.filter { ($0 == "Movie" && saved.movie) || ($0 == "Series" && saved.series) || ($0 == "Episode" && saved.episode) })
            }
            if types.isEmpty, let first = supportedTypes.first { types.insert(first) }
        }
        let available = capabilities.methods.filter { $0 != "title_only" }
        methods.formIntersection(available)
        if methods.isEmpty { methods = Set(available) }
        if !capabilities.methods.contains("title_only") { titleOnly = false }
        if titleOnly { methods.remove("semantic") }
        if methods.isDisjoint(with: ["text", "fuzzy"]), let engine = available.first(where: { $0 == "text" || $0 == "fuzzy" }) { methods.insert(engine) }
        if !capabilities.sources.contains(source) { source = capabilities.sources.first ?? "" }
    }
}

private struct StoredSettings: Codable {
    let version: Int
    let sourceID: String
    let libraryID: String
    let source: String
    let movie: Bool
    let series: Bool
    let episode: Bool
    let text: Bool
    let fuzzy: Bool
    let semantic: Bool
    let titleOnly: Bool
    let subtitle: String

    init(_ value: ClientSettings) {
        version = 1
        sourceID = value.scope?.sourceId ?? ""; libraryID = value.scope?.libraryId ?? ""
        source = value.source
        movie = value.types?.movie ?? false; series = value.types?.series ?? false; episode = value.types?.episode ?? false
        text = value.methods?.text ?? false; fuzzy = value.methods?.fuzzy ?? false
        semantic = value.methods?.semantic ?? false; titleOnly = value.methods?.titleOnly ?? false
        subtitle = value.subtitlePreference
    }

    func validated() throws -> ClientSettings {
        guard version == 1, text || fuzzy, !titleOnly || !semantic else { throw CocoaError(.coderReadCorrupt) }
        return try validateSettings(settings: ClientSettings(scope: SourceLibraryScope(sourceId: sourceID, libraryId: libraryID), source: source, types: SearchTypes(movie: movie, series: series, episode: episode), methods: SearchMethods(titleOnly: titleOnly, text: text, fuzzy: fuzzy, semantic: semantic), subtitlePreference: subtitle))
    }
}

private struct OperationTimeout: LocalizedError {
    var errorDescription: String? { "The operation timed out. Please retry." }
}

private struct OperationBudget: Sendable {
    let deadline: ContinuousClock.Instant
    init(_ timeout: Duration) { deadline = ContinuousClock.now.advanced(by: timeout) }

    func check() throws {
        try Task.checkCancellation()
        guard ContinuousClock.now < deadline else { throw OperationTimeout() }
    }

    @MainActor
    func run<T: Sendable>(_ operation: @escaping @MainActor @Sendable () async throws -> T) async throws -> T {
        try Task.checkCancellation()
        guard ContinuousClock.now < deadline else { throw OperationTimeout() }
        let value = try await withThrowingTaskGroup(of: T.self) { group in
            group.addTask { try await operation() }
            group.addTask {
                try await ContinuousClock().sleep(until: deadline)
                throw OperationTimeout()
            }
            defer { group.cancelAll() }
            let value = try await group.next()!
            try Task.checkCancellation()
            guard ContinuousClock.now < deadline else { throw OperationTimeout() }
            return value
        }
        try check()
        return value
    }
}

private struct SettingsFile {
    let url: URL
    let write: (Data, URL) throws -> Void

    init(namespace: String, url: URL?, write: ((Data, URL) throws -> Void)?) throws {
        guard namespace.utf8.count <= 64 else { throw CocoaError(.fileWriteInvalidFileName) }
        let name = namespace.utf8.map { String(format: "%02x", $0) }.joined()
        self.url = try url ?? FileManager.default.url(for: .applicationSupportDirectory, in: .userDomainMask, appropriateFor: nil, create: true)
            .appendingPathComponent("Screenroom/Settings", isDirectory: true).appendingPathComponent("settings-v1-\(name).json")
        self.write = write ?? Self.atomicWrite
    }

    static func atomicWrite(_ data: Data, _ url: URL) throws {
        let directory = url.deletingLastPathComponent()
        try FileManager.default.createDirectory(at: directory, withIntermediateDirectories: true, attributes: [.posixPermissions: 0o700])
        try FileManager.default.setAttributes([.posixPermissions: 0o700], ofItemAtPath: directory.path)
        try data.write(to: url, options: .atomic)
        try FileManager.default.setAttributes([.posixPermissions: 0o600], ofItemAtPath: url.path)
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
    var preferences: ClientSettings?
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
    var searchDegradation = ""
    var effectiveSearchMethods: SearchMethods?
    var persistenceWarning = ""
    var scope = SourceLibraryScope(sourceId: "", libraryId: "")
    private var settingsFile: SettingsFile?
    private let operationTimeout: Duration
    private struct LoadContext: Equatable {
        let screen: String
        let scope: SourceLibraryScope
        let itemID: String?
    }
    private var loadContext: LoadContext?
    private var librarySourceID: String?
    private var provenanceGeneration = 0
    private var generation = 0
    private var searchGeneration = 0
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

    init(settingsURL: URL? = nil, settingsWriter: ((Data, URL) throws -> Void)? = nil) {
        operationTimeout = .seconds(120)
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
            client = try Client(config: ClientConfig(endpoint: endpoint, userId: config("SCREENROOM_USER_ID", ""), timeoutMs: 5000))
        } catch { client = nil; configurationError = error.localizedDescription; self.error = configurationError }
        preferences = defaultSettings()
        let namespace = config("SCREENROOM_E2E_SETTINGS_NAMESPACE", "default")
        do {
            let file = try SettingsFile(namespace: namespace, url: settingsURL, write: settingsWriter)
            settingsFile = file
            if FileManager.default.fileExists(atPath: file.url.path) {
                preferences = try JSONDecoder().decode(StoredSettings.self, from: Data(contentsOf: file.url)).validated()
            } else if let legacy = UserDefaults.standard.data(forKey: "screenroom.settings.v1.\(namespace)") {
                preferences = try JSONDecoder().decode(StoredSettings.self, from: legacy).validated()
                try file.write(try JSONEncoder().encode(StoredSettings(preferences!)), file.url)
            }
        } catch { persistenceWarning = "Settings could not be loaded or saved: \(error.localizedDescription)" }
        scope = preferences?.scope ?? SourceLibraryScope(sourceId: "", libraryId: "")
    }

    init(client: Client, pageSize: UInt32 = 100, operationTimeout: Duration = .seconds(120)) {
        self.client = client
        self.pageSize = pageSize
        self.operationTimeout = operationTimeout
        preferences = defaultSettings()
    }

    func libraryTitle(_ id: String) -> String { id == "all" || id.isEmpty ? "All libraries" : libraries.first { $0.id == id }?.title ?? "Library" }

    private func collect<T: Sendable>(_ budget: OperationBudget, _ fetch: @escaping @MainActor @Sendable (Page) async throws -> ([T], String)) async throws -> [T] {
        var values: [T] = []; var token = ""; var seen: Set<String> = []
        for _ in 0..<512 {
            try Task.checkCancellation()
            let page = Page(size: pageSize, token: token)
            let (batch, next) = try await budget.run { try await fetch(page) }
            try Task.checkCancellation()
            guard values.count + batch.count <= 10000 else { throw CocoaError(.fileReadTooLarge) }
            values += batch
            if next.isEmpty { return values }
            guard seen.insert(next).inserted else { throw CocoaError(.fileReadCorruptFile) }
            token = next
        }
        throw CocoaError(.fileReadTooLarge)
    }

    func invalidate() {
        client?.invalidateReads()
        artworkRevision += 1
        loadedArtwork = []; failedArtwork = []
    }

    func load(screen: String, library: String, itemID: String?, forceBootstrap: Bool = false) async {
        let budget = OperationBudget(operationTimeout)
        generation += 1
        let current = generation
        let requestedScope = SourceLibraryScope(sourceId: scope.sourceId, libraryId: library == "all" ? "" : library)
        let context = LoadContext(screen: screen, scope: requestedScope, itemID: itemID)
        if context != loadContext {
            rows = []; items = []; results = []
            searchDegradation = ""; effectiveSearchMethods = nil
        }
        if let source = librarySourceID, source != requestedScope.sourceId { libraries = []; capabilities = nil }
        librarySourceID = requestedScope.sourceId
        loadContext = context
        detail = nil; credits = []; children = []
        loading = true; error = ""
        scope = requestedScope
        guard let client else { error = configurationError; loading = false; return }
        do {
            if libraries.isEmpty || forceBootstrap {
                let values = try await collect(budget) { page in
                    let result = try await client.listLibraries(sourceId: requestedScope.sourceId, page: page)
                    return (result.libraries, result.nextPageToken)
                }
                try Task.checkCancellation()
                let caps = try await budget.run { try await client.getSearchCapabilities() }
                try budget.check()
                guard current == generation else { return }
                libraries = values; capabilities = caps
            }
            if let itemID {
                let item = try await budget.run { try await client.getItem(id: itemID, scope: requestedScope) }
                try Task.checkCancellation()
                let people = try await collect(budget) { page in
                    let result = try await client.listCredits(itemId: itemID, page: page, scope: requestedScope)
                    return (result.credits, result.nextPageToken)
                }
                let descendants = try await collect(budget) { page in
                    let result = try await client.listChildren(parentId: itemID, page: page, scope: requestedScope)
                    return (result.items, result.nextPageToken)
                }
                let mapped = descendants.map(CatalogItem.init)
                try budget.check()
                guard current == generation else { return }
                detail = CatalogItem(value: item); credits = people; children = mapped
            } else if screen == "Home" {
                let values = try await budget.run { try await client.getHome(scope: requestedScope) }
                try budget.check()
                guard current == generation else { return }
                rows = values
            } else if screen == "Library" {
                let values = try await collect(budget) { page in
                    let result = try await client.listLibraryItems(scope: requestedScope, page: page)
                    return (result.items, result.nextPageToken)
                }
                let mapped = values.map(CatalogItem.init)
                try budget.check()
                guard current == generation else { return }
                items = mapped
            }
        } catch {
            if !Task.isCancelled && current == generation { self.error = "Unable to load: \(error.localizedDescription)" }
        }
        if current == generation { loading = false }
    }

    func loadProvenance() async {
        let budget = OperationBudget(operationTimeout)
        provenanceGeneration += 1
        let current = provenanceGeneration
        provenanceLoading = true; provenanceError = ""
        provenanceItems = []; provenanceArtwork = []
        defer { if current == provenanceGeneration { provenanceLoading = false } }
        guard let client else { provenanceError = configurationError; return }
        do {
            let info = try await budget.run { try await client.getCatalogInfo() }
            try Task.checkCancellation()
            let artwork = try await collect(budget) { page in
                let result = try await client.listArtwork(scope: SourceLibraryScope(sourceId: "", libraryId: ""), page: page)
                return (result.artwork, result.nextPageToken)
            }
            try budget.check()
            guard current == provenanceGeneration else { return }
            catalogInfo = info
            provenanceArtwork = artwork
        } catch {
            if !Task.isCancelled && current == provenanceGeneration { provenanceError = "Unable to load credits: \(error.localizedDescription)" }
        }
    }

    func search(query: String, library: String, options: SearchOptions) async {
        let budget = OperationBudget(operationTimeout)
        searchGeneration += 1
        let current = searchGeneration
        results = []
        searchDegradation = ""; effectiveSearchMethods = nil
        guard !query.trimmingCharacters(in: .whitespaces).isEmpty, let client else { return }
        let requestedScope = SourceLibraryScope(sourceId: scope.sourceId, libraryId: library == "all" ? "" : library)
        do {
            try await budget.run { try await Task.sleep(for: .milliseconds(350)) }
            var degradation = ""
            var effective: SearchMethods?
            let values = try await collect(budget) { page in
                let result = try await client.search(request: SearchRequest(query: query, libraryId: requestedScope.libraryId, types: options.coreTypes, methods: options.coreMethods, source: options.source, page: page, sourceId: requestedScope.sourceId))
                if !result.degradationReason.isEmpty { degradation = result.degradationReason }
                effective = result.effectiveMethods
                return (result.items, result.nextPageToken)
            }
            let mapped = values.map(CatalogItem.init)
            try budget.check()
            guard current == searchGeneration else { return }
            results = mapped; searchDegradation = degradation; effectiveSearchMethods = effective; error = ""
        } catch {
            if !Task.isCancelled && current == searchGeneration { self.error = "Search unavailable: \(error.localizedDescription)" }
        }
    }

    func save(options: SearchOptions, library: String, subtitle: String) {
        let value = ClientSettings(scope: SourceLibraryScope(sourceId: scope.sourceId, libraryId: library == "all" ? "" : library), source: options.source, types: options.coreTypes, methods: options.coreMethods, subtitlePreference: subtitle)
        do {
            let saved = try StoredSettings(value).validated()
            preferences = saved
            retrySettingsSave()
        } catch { persistenceWarning = "Settings not saved: \(error.localizedDescription)" }
    }

    func retrySettingsSave() {
        do {
            guard let file = settingsFile, let preferences else { throw CocoaError(.fileWriteUnknown) }
            try file.write(try JSONEncoder().encode(StoredSettings(preferences)), file.url)
            persistenceWarning = ""
        } catch { persistenceWarning = "Settings not saved: \(error.localizedDescription). Your current choices remain available." }
    }

    func ancestors(for id: String) async throws -> [CatalogItem] {
        let budget = OperationBudget(operationTimeout)
        guard let client else { throw CocoaError(.fileReadUnknown) }
        let requestedScope = scope
        var value = try await budget.run { try await client.getItem(id: id, scope: requestedScope) }
        var values = [CatalogItem(value: value)]
        var seen: Set<String> = [value.id]
        while let parent = value.parentId {
            guard seen.count < 512, seen.insert(parent).inserted else { throw CocoaError(.fileReadCorruptFile) }
            value = try await budget.run { try await client.getItem(id: parent, scope: requestedScope) }
            values.insert(CatalogItem(value: value), at: 0)
        }
        try Task.checkCancellation()
        guard ContinuousClock.now < budget.deadline else { throw OperationTimeout() }
        return values
    }
}
