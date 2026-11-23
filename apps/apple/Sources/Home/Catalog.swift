import Foundation
import Observation

struct MetadataRecord {
    let date: String
    let rating: Rating?
    let providers: [ProviderReference]
    let retrievedAt: String
    var score: String {
        guard let rating, rating.value.isFinite, rating.scale.isFinite, rating.scale > 0,
              rating.value >= 0, rating.value <= rating.scale else { return "—" }
        return rating.votes == 0 ? "Unrated" : String(format: "%.1f/%g", rating.value, rating.scale)
    }
    var ratingDetail: String {
        guard let rating else { return "Rating snapshot not imported." }
        return "\(rating.provider) user rating: \(score) · \(rating.votes.map { "\($0) votes" } ?? "Vote count not available")."
    }
}

func externalURL(_ value: String) -> URL? {
    guard let url = URL(string: value), ["https", "http"].contains(url.scheme?.lowercased()),
          url.host?.isEmpty == false, url.user == nil, url.password == nil,
          !value.unicodeScalars.contains(where: { CharacterSet.controlCharacters.contains($0) }) else { return nil }
    return url
}

func providerStatus(_ status: String) -> String {
    switch status {
    case "source_supplied": "Identifier supplied by Jellyfin"
    case "verified": "Verified"
    case "matched": "Matched"
    case "unverified": "Unverified"
    case "unavailable": "Unavailable"
    default: status.replacingOccurrences(of: "_", with: " ").capitalized
    }
}

func choiceLabel(_ value: String) -> String {
    switch value {
    case "all": "All catalogs"
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
    var watched: Bool { value.watched == true }
    var playbackAccessibility: String {
        ([watched ? "Watched" : nil, value.progressPercent.map { "Playback progress \($0)%" }].compactMap { $0 }).joined(separator: ", ")
    }
    var cardAccessibility: String { [title, kind, meta, playbackAccessibility].filter { !$0.isEmpty }.joined(separator: ", ") }
    func labels(seasonContext: Bool = false) -> CardLabels {
        guard kind == "Episode" else { return CardLabels(primary: title, number: "", subtitle: kind == "Movie" ? String(year) : meta) }
        let pattern = #"^(?:(.+) · )?S(\d+)\s*E(\d+)(?: · .*)?$"#
        let trimmed = meta.trimmingCharacters(in: .whitespacesAndNewlines)
        guard let regex = try? NSRegularExpression(pattern: pattern),
              let match = regex.firstMatch(in: trimmed, range: NSRange(trimmed.startIndex..., in: trimmed)) else {
            return CardLabels(primary: "Episode", number: "", subtitle: title)
        }
        let text = trimmed as NSString
        func part(_ n: Int) -> String { match.range(at: n).location == NSNotFound ? "" : text.substring(with: match.range(at: n)) }
        func padded(_ value: String) -> String { value.count < 2 ? "0" + value : value }
        let episode = "E" + padded(part(3))
        return CardLabels(primary: seasonContext ? "" : part(1), number: seasonContext ? episode : "S" + padded(part(2)) + ":" + episode, subtitle: title)
    }
    func isSeasonChild(of parent: CatalogItem?) -> Bool {
        parent?.kind == "Season" && kind == "Episode" && self.parent == parent?.id
    }
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

struct CardLabels: Equatable {
    let primary: String
    let number: String
    let subtitle: String
    var heading: String { [primary, number].filter { !$0.isEmpty }.joined(separator: " · ") }
}

struct CatalogNavigationKey: Hashable {
    let screen: String
    let library: String
    let itemID: String
    let query: String
    let source: String
    let types: [String]
    let methods: [String]
    let titleOnly: Bool
    var sourceID: String = ""
}

struct CatalogNavigationMemory {
    private struct Entry {
        var anchor: String?
        var focus: String?
    }
    private var entries: [CatalogNavigationKey: Entry] = [:]
    private var order: [CatalogNavigationKey] = []
    func anchor(for key: CatalogNavigationKey) -> String? { entries[key]?.anchor }
    func focus(for key: CatalogNavigationKey) -> String? { entries[key]?.focus }
    mutating func forget(_ key: CatalogNavigationKey) {
        entries.removeValue(forKey: key)
        order.removeAll { $0 == key }
    }
    mutating func remember(_ key: CatalogNavigationKey, anchor: String? = nil, focus: String? = nil) {
        var entry = entries[key] ?? Entry()
        if let anchor { entry.anchor = anchor }
        if let focus { entry.focus = focus }
        entries[key] = entry
        order.removeAll { $0 == key }
        order.append(key)
        while order.count > 64 { entries.removeValue(forKey: order.removeFirst()) }
    }
}

func loadedItemCount(_ count: Int) -> String { "\(count) \(count == 1 ? "item" : "items") loaded" }

func homeRowError(_ code: String) -> String {
    code == "progress_unavailable"
        ? "Continue Watching is unavailable. Jellyfin progress could not be read. Other catalog rows remain available."
        : "This row could not be loaded. Other catalog rows remain available."
}

struct ItemPagination {
    var token = ""
    var seen: Set<String> = []
    var pages = 0
    mutating func append(_ batch: [Item], next: String, to values: [CatalogItem]) throws -> [CatalogItem] {
        guard pages < 512, values.count + batch.count <= 10000,
              next.isEmpty || !seen.contains(next) else { throw CocoaError(.fileReadCorruptFile) }
        var ids = Set(values.map(\.id))
        let appended = batch.filter { ids.insert($0.id).inserted }.map(CatalogItem.init)
        pages += 1
        if !next.isEmpty { seen.insert(next) }
        token = next
        return values + appended
    }
}

@MainActor
final class ArtworkLoader {
    private struct Job {
        let fetch: @Sendable () async throws -> Data
        var waiters: [UUID: CheckedContinuation<Data, Error>]
        var task: Task<Void, Never>?
    }
    private var jobs: [String: Job] = [:]
    private var queue: [String] = []
    private var active = 0
    private var cache: [String: Data] = [:]
    private var cacheOrder: [String] = []
    private var cacheBytes = 0
    private(set) var peakActive = 0

    func load(key: String, fetch: @escaping @Sendable () async throws -> Data) async throws -> Data {
        try Task.checkCancellation()
        if let data = cache[key] { return data }
        if let task = jobs[key]?.task, task.isCancelled {
            await task.value
            try Task.checkCancellation()
            return try await load(key: key, fetch: fetch)
        }
        let id = UUID()
        return try await withTaskCancellationHandler {
            try await withCheckedThrowingContinuation { continuation in
                if Task.isCancelled { continuation.resume(throwing: CancellationError()); return }
                if jobs[key] != nil { jobs[key]!.waiters[id] = continuation }
                else {
                    jobs[key] = Job(fetch: fetch, waiters: [id: continuation])
                    queue.append(key)
                }
                pump()
            }
        } onCancel: {
            Task { @MainActor in self.cancel(key: key, id: id) }
        }
    }

    private func cancel(key: String, id: UUID) {
        guard var job = jobs[key], let waiter = job.waiters.removeValue(forKey: id) else { return }
        waiter.resume(throwing: CancellationError())
        jobs[key] = job
        if job.waiters.isEmpty {
            if let task = job.task { task.cancel() }
            else { jobs.removeValue(forKey: key); queue.removeAll { $0 == key } }
        }
    }

    private func pump() {
        while active < 8, !queue.isEmpty {
            let key = queue.removeFirst()
            guard let job = jobs[key] else { continue }
            active += 1; peakActive = max(peakActive, active)
            jobs[key]!.task = Task {
                let result: Result<Data, Error>
                do { result = .success(try await job.fetch()) }
                catch { result = .failure(error) }
                finish(key: key, result: result)
            }
        }
    }

    private func finish(key: String, result: Result<Data, Error>) {
        guard let job = jobs.removeValue(forKey: key) else { return }
        active -= 1
        if case let .success(data) = result, !job.waiters.isEmpty, data.count <= 32 * 1024 * 1024 {
            cache[key] = data; cacheOrder.append(key); cacheBytes += data.count
            while cacheOrder.count > 128 || cacheBytes > 32 * 1024 * 1024 {
                let oldest = cacheOrder.removeFirst()
                cacheBytes -= cache.removeValue(forKey: oldest)?.count ?? 0
            }
        }
        for waiter in job.waiters.values { waiter.resume(with: result) }
        pump()
    }
}

@MainActor @Observable
final class JellyfinProfileStore {
    var sources: [JellyfinSource] = []
    var profiles: [JellyfinProfile] = []
    var source = ""
    var selected = ""
    var link: JellyfinLink?
    var known = false
    var busy = false
    var error = ""
    var status = ""
    var revision = 0
    private var pendingReconciliation = false
    private var writeInFlight = false
    private var generation = 0
    func deactivate() {
        generation += 1
        busy = false
        known = false
    }
    var currentProfile: String {
        guard known else { return "Not confirmed" }
        guard let link else { return "Not linked" }
        return profiles.first { $0.id == link.jellyfinUserId }?.displayName ?? "Linked profile unavailable in choices"
    }

    private func collect<T: Sendable>(_ fetch: @escaping @MainActor @Sendable (Page) async throws -> ([T], String), id: (T) -> String) async throws -> [T] {
        let budget = OperationBudget(.seconds(120))
        var values: [T] = []; var token = ""; var tokens: Set<String> = []; var ids: Set<String> = []
        for _ in 0..<512 {
            let page = Page(size: 100, token: token)
            let (batch, next) = try await budget.run { try await fetch(page) }
            guard values.count + batch.count <= 10000 else { throw CocoaError(.fileReadTooLarge) }
            values += batch.filter { ids.insert(id($0)).inserted }
            if next.isEmpty { return values }
            guard tokens.insert(next).inserted else { throw CocoaError(.fileReadCorruptFile) }
            token = next
        }
        throw CocoaError(.fileReadTooLarge)
    }

    func load(client: Client?, target: String? = nil) async {
        guard !busy, !Task.isCancelled else { return }
        generation += 1
        let current = generation
        busy = true; known = false; error = ""; status = ""; profiles = []
        defer { if current == generation { busy = false } }
        do {
            guard let client else { throw CocoaError(.fileReadUnknown) }
            guard !writeInFlight else {
                self.error = "A previous profile change is still settling. Retry reads server state only."
                return
            }
            client.invalidateReads()
            let values = try await collect({ page in
                let result = try await client.listJellyfinSources(page: page)
                return (result.sources, result.nextPageToken)
            }, id: { $0.id })
            try Task.checkCancellation()
            guard current == generation else { return }
            sources = values
            let wanted = target ?? source
            source = values.contains { $0.id == wanted } ? wanted : values.first?.id ?? ""
            guard !source.isEmpty else { return }
            let requested = source
            let choices = try await collect({ page in
                let result = try await client.listJellyfinProfiles(sourceId: requested, page: page)
                return (result.profiles, result.nextPageToken)
            }, id: { $0.id })
            let mapping = try await OperationBudget(.seconds(120)).run { try await client.getJellyfinLink(sourceId: requested) }
            try Task.checkCancellation()
            guard current == generation else { return }
            profiles = choices; link = mapping; selected = mapping?.jellyfinUserId ?? choices.first?.id ?? ""; known = true
            if pendingReconciliation { pendingReconciliation = false; revision += 1 }
        } catch {
            if !Task.isCancelled && current == generation { self.error = "Unable to read Jellyfin profile settings. Retry to read the server state." }
        }
    }

    func change(client: Client?, profile: String?) async {
        guard let client, known, !busy, !source.isEmpty, !Task.isCancelled else { return }
        if let profile, !profiles.contains(where: { $0.id == profile }) { return }
        generation += 1
        let current = generation
        busy = true; known = false; error = ""; status = ""
        pendingReconciliation = true
        let requested = source
        defer { if current == generation { busy = false } }
        var ambiguous = false
        var validReturn = false
        writeInFlight = true
        do {
            if let profile {
                let returned = try await client.setJellyfinLink(sourceId: requested, jellyfinUserId: profile)
                validReturn = returned.sourceId == requested && returned.jellyfinUserId == profile
            } else { validReturn = try await client.clearJellyfinLink(sourceId: requested) == requested }
        } catch { ambiguous = true }
        writeInFlight = false
        // A failed write is ambiguous: only a fresh read may reconcile it.
        guard current == generation, !Task.isCancelled else { return }
        client.invalidateReads()
        do {
            let mapping = try await OperationBudget(.seconds(120)).run { try await client.getJellyfinLink(sourceId: requested) }
            try Task.checkCancellation()
            guard current == generation else { return }
            link = mapping; known = true; selected = mapping?.jellyfinUserId ?? profiles.first?.id ?? ""
            let confirmed = profile == nil ? mapping == nil : mapping?.sourceId == requested && mapping?.jellyfinUserId == profile
            if confirmed && (ambiguous || validReturn) {
                status = ambiguous ? "Server state checked: profile change is saved." : "Profile change saved on the server."
            } else { error = "The requested change was not confirmed. Current server state is shown; choose a profile before trying again." }
            pendingReconciliation = false; revision += 1
        } catch {
            if current == generation && !Task.isCancelled { self.error = "Profile change could not be confirmed. Retry reads server state only; the change will not be repeated." }
        }
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
    init(_ value: ClientSettings) {
        version = 1
        sourceID = value.scope?.sourceId ?? ""; libraryID = value.scope?.libraryId ?? ""
        source = value.source
        movie = value.types?.movie ?? false; series = value.types?.series ?? false; episode = value.types?.episode ?? false
        text = value.methods?.text ?? false; fuzzy = value.methods?.fuzzy ?? false
        semantic = value.methods?.semantic ?? false; titleOnly = value.methods?.titleOnly ?? false
    }

    func validated() throws -> ClientSettings {
        guard version == 1, text || fuzzy, !titleOnly || !semantic else { throw CocoaError(.coderReadCorrupt) }
        return try normalizeSavedSettings(settings: ClientSettings(scope: SourceLibraryScope(sourceId: sourceID, libraryId: libraryID), source: source, types: SearchTypes(movie: movie, series: series, episode: episode), methods: SearchMethods(titleOnly: titleOnly, text: text, fuzzy: fuzzy, semantic: semantic)))
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
    let profileClient: Client?
    let jellyfinProfile = JellyfinProfileStore()
    let artworkLoader = ArtworkLoader()
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
    var error = ""
    var loading = false
    var loadedArtwork: Set<String> = []
    var failedArtwork: Set<String> = []
    var artworkRevision = 0
    var searchDegradation = ""
    var effectiveSearchMethods: SearchMethods?
    var pageError = ""
    var pageLoading = false
    var itemPage = ItemPagination()
    var searchPage = ItemPagination()
    private var pageGeneration = 0
    private var searchRequest: SearchRequest?
    var persistenceWarning = ""
    var scope = SourceLibraryScope(sourceId: "", libraryId: "") {
        didSet {
            if oldValue.sourceId != scope.sourceId {
                discardNavigationEntries(clearPublished: true)
            }
        }
    }
    var navigationMemory = CatalogNavigationMemory()
    private(set) var navigationRestorationRevision = 0
    private var settingsFile: SettingsFile?
    private let operationTimeout: Duration
    private struct LoadContext: Equatable {
        let screen: String
        let scope: SourceLibraryScope
        let itemID: String?
    }
    private var loadContext: LoadContext?
    private var librarySourceID: String?
    private var generation = 0
    private var searchGeneration = 0
    private var homeProfileRevision = 0
    private var configurationError = ""
    private struct EntrySnapshot {
        let context: LoadContext?
        let rows: [HomeRow]
        let items: [CatalogItem]
        let results: [CatalogItem]
        let detail: CatalogItem?
        let children: [CatalogItem]
        let credits: [Credit]
        let itemPage: ItemPagination
        let searchPage: ItemPagination
        let searchRequest: SearchRequest?
        let error: String
        let pageError: String
        let degradation: String
        let effectiveMethods: SearchMethods?
        let profileRevision: Int
        let epoch: Int
        var count: Int { rows.reduce(0) { $0 + $1.items.count } + items.count + results.count + children.count + credits.count + (detail == nil ? 0 : 1) }
    }
    private var entrySnapshots: [CatalogNavigationKey: EntrySnapshot] = [:]
    private var entryOrder: [CatalogNavigationKey] = []
    private var activeEntry: CatalogNavigationKey?
    private var activeEntryReady = false
    private var entryEpoch = 0
    private var entryProfileRevision = 0
    private var visitGeneration = 0

    private func discardNavigationEntries(clearPublished: Bool = false) {
        entryEpoch += 1
        visitGeneration += 1
        generation += 1
        searchGeneration += 1
        pageGeneration += 1
        entrySnapshots = [:]; entryOrder = []; activeEntry = nil; activeEntryReady = false
        navigationMemory = CatalogNavigationMemory()
        if clearPublished {
            rows = []; items = []; results = []; children = []; credits = []; detail = nil
            itemPage = ItemPagination(); searchPage = ItemPagination(); searchRequest = nil
            loadContext = nil
            pageError = ""; error = ""; searchDegradation = ""; effectiveSearchMethods = nil
            loading = false; pageLoading = false
        }
    }

    private func retainActiveEntry() {
        guard activeEntryReady, let key = activeEntry,
              entryProfileRevision == jellyfinProfile.revision, key.sourceID == scope.sourceId else { return }
        let snapshot = EntrySnapshot(context: loadContext, rows: rows, items: items, results: results,
            detail: detail, children: children, credits: credits, itemPage: itemPage, searchPage: searchPage,
            searchRequest: searchRequest, error: error, pageError: pageError, degradation: searchDegradation,
            effectiveMethods: effectiveSearchMethods, profileRevision: jellyfinProfile.revision, epoch: entryEpoch)
        entrySnapshots[key] = snapshot
        entryOrder.removeAll { $0 == key }; entryOrder.append(key)
        while entryOrder.count > 16 || entrySnapshots.values.reduce(0, { $0 + $1.count }) > 10000 {
            let evicted = entryOrder.removeFirst()
            entrySnapshots.removeValue(forKey: evicted)
            navigationMemory.forget(evicted)
        }
    }

    @discardableResult
    func visit(_ key: CatalogNavigationKey, options: SearchOptions, forceBootstrap: Bool = false) async -> Bool {
        guard key.sourceID == scope.sourceId, key.source == options.source,
              key.types == options.types.sorted(), key.methods == options.methods.sorted(),
              key.titleOnly == options.titleOnly else { return false }
        let profile = jellyfinProfile.revision
        if forceBootstrap || entryProfileRevision != profile {
            discardNavigationEntries(clearPublished: entryProfileRevision != profile)
            client?.invalidateReads()
        } else {
            retainActiveEntry()
        }
        entryProfileRevision = profile
        visitGeneration += 1
        let current = visitGeneration
        let epoch = entryEpoch
        generation += 1; searchGeneration += 1; pageGeneration += 1
        pageLoading = false
        activeEntry = key
        activeEntryReady = false
        if let snapshot = entrySnapshots[key], snapshot.epoch == epoch, snapshot.profileRevision == profile {
            loadContext = snapshot.context
            scope = snapshot.context?.scope ?? SourceLibraryScope(sourceId: key.sourceID, libraryId: key.library == "all" ? "" : key.library)
            rows = snapshot.rows; items = snapshot.items; results = snapshot.results
            detail = snapshot.detail; children = snapshot.children; credits = snapshot.credits
            itemPage = snapshot.itemPage; searchPage = snapshot.searchPage; searchRequest = snapshot.searchRequest
            error = snapshot.error; pageError = snapshot.pageError
            searchDegradation = snapshot.degradation; effectiveSearchMethods = snapshot.effectiveMethods
            loading = false; activeEntryReady = true
            navigationRestorationRevision += 1
            return true
        }
        await load(screen: key.screen, library: key.library, itemID: key.itemID.isEmpty ? nil : key.itemID, forceBootstrap: forceBootstrap)
        guard !Task.isCancelled, current == visitGeneration, epoch == entryEpoch,
              profile == jellyfinProfile.revision, key.sourceID == scope.sourceId else { return false }
        if key.screen == "Search" && key.itemID.isEmpty && error.isEmpty {
            await search(query: key.query, library: key.library, options: options)
        }
        guard !Task.isCancelled, current == visitGeneration, epoch == entryEpoch,
              profile == jellyfinProfile.revision, key.sourceID == scope.sourceId else { return false }
        activeEntryReady = true
        return true
    }

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
            let endpoint = config("SCREENROOM_API_ENDPOINT", "https://preview-screenroom.holdenitdown.net")
            try Self.validateEndpoint(endpoint)
            let context = ClientConfig(endpoint: endpoint, userId: config("SCREENROOM_USER_ID", ""), timeoutMs: 5000)
            let reads = try Client(config: context)
            // Core mutation settlement invalidates reads on its owning client.
            let profiles = try Client(config: context)
            client = reads
            profileClient = profiles
        } catch { client = nil; profileClient = nil; configurationError = error.localizedDescription; self.error = configurationError }
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

    init(client: Client, profileClient: Client? = nil, pageSize: UInt32 = 100, operationTimeout: Duration = .seconds(120), settingsURL: URL? = nil, settingsWriter: ((Data, URL) throws -> Void)? = nil) {
        precondition(profileClient == nil || profileClient !== client)
        self.client = client
        self.profileClient = profileClient
        self.pageSize = pageSize
        self.operationTimeout = operationTimeout
        preferences = defaultSettings()
        if let settingsURL {
            do {
                settingsFile = try SettingsFile(namespace: ProcessInfo.processInfo.environment["SCREENROOM_E2E_SETTINGS_NAMESPACE"] ?? "default", url: settingsURL, write: settingsWriter)
            } catch { persistenceWarning = "Settings could not be loaded or saved: \(error.localizedDescription)" }
        }
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
        discardNavigationEntries()
        client?.invalidateReads()
        artworkRevision += 1
        loadedArtwork = []; failedArtwork = []
    }

    func load(screen: String, library: String, itemID: String?, forceBootstrap: Bool = false) async {
        let budget = OperationBudget(operationTimeout)
        let profileRevision = jellyfinProfile.revision
        generation += 1
        searchGeneration += 1
        pageGeneration += 1
        pageLoading = false; pageError = ""; itemPage = ItemPagination(); searchPage = ItemPagination(); searchRequest = nil
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
                guard current == generation, profileRevision == jellyfinProfile.revision else { return }
                libraries = values; capabilities = caps
            }
            if let itemID {
                let item = try await budget.run { try await client.getItem(id: itemID, scope: requestedScope) }
                try Task.checkCancellation()
                let people = try await collect(budget) { page in
                    let result = try await client.listCredits(itemId: itemID, page: page, scope: requestedScope)
                    return (result.credits, result.nextPageToken)
                }
                let descendants = try await budget.run { try await client.listChildren(parentId: itemID, page: Page(size: self.pageSize, token: ""), scope: requestedScope) }
                try budget.check()
                guard current == generation, profileRevision == jellyfinProfile.revision else { return }
                let mapped = try itemPage.append(descendants.items, next: descendants.nextPageToken, to: [])
                detail = CatalogItem(value: item); credits = people; children = mapped
            } else if screen == "Home" {
                try budget.check()
                guard current == generation, profileRevision == jellyfinProfile.revision else { return }
                let profileRevision = jellyfinProfile.revision
                if homeProfileRevision != profileRevision { client.invalidateReads() }
                let values = try await budget.run { try await client.getHome(scope: requestedScope) }
                try budget.check()
                guard current == generation, profileRevision == jellyfinProfile.revision else { return }
                rows = values
                homeProfileRevision = profileRevision
            } else if screen == "Library" {
                let values = try await budget.run { try await client.listLibraryItems(scope: requestedScope, page: Page(size: self.pageSize, token: "")) }
                try budget.check()
                guard current == generation, profileRevision == jellyfinProfile.revision else { return }
                items = try itemPage.append(values.items, next: values.nextPageToken, to: [])
            }
        } catch {
            if !Task.isCancelled && current == generation && profileRevision == jellyfinProfile.revision { self.error = "Unable to load: \(error.localizedDescription)" }
        }
        if current == generation { loading = false }
    }

    func search(query: String, library: String, options: SearchOptions) async {
        let budget = OperationBudget(operationTimeout)
        let profileRevision = jellyfinProfile.revision
        searchGeneration += 1
        pageGeneration += 1
        pageLoading = false; pageError = ""; searchPage = ItemPagination(); searchRequest = nil
        let current = searchGeneration
        results = []
        searchDegradation = ""; effectiveSearchMethods = nil
        guard !query.trimmingCharacters(in: .whitespaces).isEmpty, let client else { return }
        let requestedScope = SourceLibraryScope(sourceId: scope.sourceId, libraryId: library == "all" ? "" : library)
        do {
            try await budget.run { try await Task.sleep(for: .milliseconds(350)) }
            let request = SearchRequest(query: query, libraryId: requestedScope.libraryId, types: options.coreTypes, methods: options.coreMethods, source: options.source, page: Page(size: pageSize, token: ""), sourceId: requestedScope.sourceId)
            let values = try await budget.run { try await client.search(request: request) }
            try budget.check()
            guard current == searchGeneration, profileRevision == jellyfinProfile.revision else { return }
            results = try searchPage.append(values.items, next: values.nextPageToken, to: [])
            searchRequest = request; searchDegradation = values.degradationReason; effectiveSearchMethods = values.effectiveMethods; error = ""
        } catch {
            if !Task.isCancelled && current == searchGeneration && profileRevision == jellyfinProfile.revision { self.error = "Search unavailable: \(error.localizedDescription)" }
        }
    }

    func loadMore(search: Bool = false) async {
        let profileRevision = jellyfinProfile.revision
        guard !pageLoading, let client, !(search ? searchPage.token : itemPage.token).isEmpty else { return }
        let current = pageGeneration
        let context = loadContext
        let requestedScope = scope
        let token = search ? searchPage.token : itemPage.token
        pageLoading = true; pageError = ""
        let budget = OperationBudget(operationTimeout)
        do {
            if search, var request = searchRequest {
                request.page = Page(size: pageSize, token: token)
                let sent = request
                let page = try await budget.run { try await client.search(request: sent) }
                try budget.check()
                guard current == pageGeneration, profileRevision == jellyfinProfile.revision else { return }
                results = try searchPage.append(page.items, next: page.nextPageToken, to: results)
                if !page.degradationReason.isEmpty { searchDegradation = page.degradationReason }
                effectiveSearchMethods = page.effectiveMethods
            } else if let context {
                let page = try await budget.run {
                    if let id = context.itemID { return try await client.listChildren(parentId: id, page: Page(size: self.pageSize, token: token), scope: requestedScope) }
                    return try await client.listLibraryItems(scope: requestedScope, page: Page(size: self.pageSize, token: token))
                }
                try budget.check()
                guard current == pageGeneration, profileRevision == jellyfinProfile.revision else { return }
                if context.itemID != nil { children = try itemPage.append(page.items, next: page.nextPageToken, to: children) }
                else { items = try itemPage.append(page.items, next: page.nextPageToken, to: items) }
            }
        } catch {
            if !Task.isCancelled && current == pageGeneration && profileRevision == jellyfinProfile.revision { pageError = "More items could not be loaded. Your current items are still available." }
        }
        if current == pageGeneration { pageLoading = false }
    }

    func retryHomeRows() async {
        guard let client else { return }
        let profileRevision = jellyfinProfile.revision
        let current = generation
        let requestedScope = scope
        client.invalidateReads()
        do {
            let values = try await OperationBudget(operationTimeout).run { try await client.getHome(scope: requestedScope) }
            guard current == generation, profileRevision == jellyfinProfile.revision, !Task.isCancelled else { return }
            rows = values
        } catch {
            // Keep existing rows, including their sanitized retry state.
        }
    }

    func save(options: SearchOptions, library: String) {
        let value = ClientSettings(scope: SourceLibraryScope(sourceId: scope.sourceId, libraryId: library == "all" ? "" : library), source: options.source, types: options.coreTypes, methods: options.coreMethods)
        do {
            let saved = try validateSettings(settings: value)
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
