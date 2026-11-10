import Foundation
import AppKit

private func testItem(_ id: String) -> Item {
    Item(id: id, title: id, kind: "Movie", parentId: nil, year: 2000, subtitle: "", overview: "", tags: [], colors: [], releaseDate: "", addedOrder: 0, titleRights: nil, metadataRights: nil, artwork: nil, rating: nil, runtime: nil, providerReferences: [], progressPercent: nil, provenanceNotes: [], metadataRetrievedAt: "")
}

private actor CreditGate {
    private var requests: [String: CheckedContinuation<CreditPage, Error>] = [:]
    private var scopes: [String: SourceLibraryScope] = [:]
    func read(_ id: String, scope: SourceLibraryScope) async throws -> CreditPage {
        scopes[id] = scope
        return try await withCheckedThrowingContinuation { requests[id] = $0 }
    }
    func requestedScope(_ id: String) -> SourceLibraryScope? { scopes[id] }
    func pending(_ id: String) -> Bool { requests[id] != nil }
    func finish(_ id: String, fail: Bool = false) {
        let continuation = requests.removeValue(forKey: id)!
        if fail { continuation.resume(throwing: CocoaError(.fileReadUnknown)) }
        else { continuation.resume(returning: CreditPage(credits: [Credit(id: id, name: id, role: "Director", department: "crew", artwork: nil)], nextPageToken: "")) }
    }
}

private final class DelayedClient: Client, @unchecked Sendable {
    let gate: CreditGate
    init(gate: CreditGate) { self.gate = gate; super.init(noHandle: NoHandle()) }
    required init(unsafeFromHandle handle: UInt64) { fatalError("DelayedClient is a test-only transport") }
    override func getItem(id: String, scope: SourceLibraryScope) async throws -> Item { testItem(id) }
    override func listCredits(itemId: String, page: Page, scope: SourceLibraryScope) async throws -> CreditPage { try await gate.read(itemId, scope: scope) }
    override func listChildren(parentId: String, page: Page, scope: SourceLibraryScope) async throws -> ItemPage { ItemPage(items: [], nextPageToken: "") }
}

private final class LoopingClient: Client, @unchecked Sendable {
    init() { super.init(noHandle: NoHandle()) }
    required init(unsafeFromHandle handle: UInt64) { fatalError("LoopingClient is a test-only transport") }
    override func listLibraries(sourceId: String, page: Page) async throws -> LibraryPage {
        LibraryPage(libraries: [MediaLibrary(id: "partial", title: "Partial")], nextPageToken: "cycle")
    }
    override func getCatalogInfo() async throws -> CatalogInfo { CatalogInfo(notices: [], retrievedAt: "", branding: []) }
    override func listArtwork(scope: SourceLibraryScope, page: Page) async throws -> ArtworkPage { ArtworkPage(artwork: [], nextPageToken: "cycle") }
}

private actor ReadProbe {
    let delays: [String: Duration]
    let failures: Set<String>
    private(set) var calls: [String] = []
    private(set) var cancelled: [String] = []
    init(_ delays: [String: Duration], failures: Set<String> = []) { self.delays = delays; self.failures = failures }
    func pause(_ key: String) async throws {
        calls.append(key)
        do { try await Task.sleep(for: delays[key] ?? .zero) }
        catch { cancelled.append(key); throw error }
        if failures.contains(key) { throw CocoaError(.fileReadUnknown) }
    }
}

private final class TimedClient: Client, @unchecked Sendable {
    let probe: ReadProbe
    init(_ probe: ReadProbe) { self.probe = probe; super.init(noHandle: NoHandle()) }
    required init(unsafeFromHandle handle: UInt64) { fatalError("Test-only transport") }
    override func listLibraries(sourceId: String, page: Page) async throws -> LibraryPage {
        try await probe.pause("libraries")
        return LibraryPage(libraries: [MediaLibrary(id: "movies", title: "Movies")], nextPageToken: "")
    }
    override func getSearchCapabilities() async throws -> SearchCapabilities {
        try await probe.pause("capabilities")
        return SearchCapabilities(methods: ["text"], types: ["movie"], sources: ["all"], semanticDescription: "", maximumQueryLength: 100)
    }
    override func getHome(scope: SourceLibraryScope) async throws -> [HomeRow] { try await probe.pause("home"); return [] }
    override func getItem(id: String, scope: SourceLibraryScope) async throws -> Item {
        try await probe.pause("item:\(id)")
        var item = testItem(id)
        if id == "child" { item.parentId = "parent" }
        return item
    }
    override func listCredits(itemId: String, page: Page, scope: SourceLibraryScope) async throws -> CreditPage {
        try await probe.pause("credits")
        return CreditPage(credits: [], nextPageToken: "")
    }
    override func listChildren(parentId: String, page: Page, scope: SourceLibraryScope) async throws -> ItemPage {
        try await probe.pause("children")
        return ItemPage(items: [], nextPageToken: "")
    }
    override func getCatalogInfo() async throws -> CatalogInfo {
        try await probe.pause("info")
        return CatalogInfo(notices: [], retrievedAt: "test", branding: [])
    }
    override func listArtwork(scope: SourceLibraryScope, page: Page) async throws -> ArtworkPage {
        try await probe.pause("artwork")
        return ArtworkPage(artwork: [], nextPageToken: "")
    }
    override func search(request: SearchRequest) async throws -> SearchPage {
        let token = request.page?.token ?? ""
        try await probe.pause("search:\(token)")
        return SearchPage(items: [testItem(token)], nextPageToken: token.isEmpty ? "last" : "", effectiveMethods: request.methods, degradationReason: "")
    }
}

private final class ContextClient: Client, @unchecked Sendable {
    let actual: Client
    let probe: ReadProbe
    init(_ actual: Client, _ probe: ReadProbe) { self.actual = actual; self.probe = probe; super.init(noHandle: NoHandle()) }
    required init(unsafeFromHandle handle: UInt64) { fatalError("Test-only transport") }
    override func listLibraries(sourceId: String, page: Page) async throws -> LibraryPage { try await actual.listLibraries(sourceId: sourceId, page: page) }
    override func getSearchCapabilities() async throws -> SearchCapabilities { try await actual.getSearchCapabilities() }
    override func listLibraryItems(scope: SourceLibraryScope, page: Page) async throws -> ItemPage {
        let response = try await actual.listLibraryItems(scope: scope, page: page)
        try await probe.pause("library:\(scope.libraryId)")
        return response
    }
    override func getHome(scope: SourceLibraryScope) async throws -> [HomeRow] {
        let response = try await actual.getHome(scope: scope)
        try await probe.pause("home:\(scope.sourceId)")
        return response
    }
    override func getItem(id: String, scope: SourceLibraryScope) async throws -> Item {
        let response = try await actual.getItem(id: id, scope: scope)
        try await probe.pause("item:\(id)")
        return response
    }
    override func listCredits(itemId: String, page: Page, scope: SourceLibraryScope) async throws -> CreditPage { try await actual.listCredits(itemId: itemId, page: page, scope: scope) }
    override func listChildren(parentId: String, page: Page, scope: SourceLibraryScope) async throws -> ItemPage { try await actual.listChildren(parentId: parentId, page: page, scope: scope) }
}

@main
private struct CatalogStateTests {
    static func wait(_ probe: ReadProbe, _ key: String) async {
        let deadline = ContinuousClock.now.advanced(by: .seconds(5))
        while ContinuousClock.now < deadline {
            if await probe.calls.contains(key) { return }
            try? await Task.sleep(for: .milliseconds(1))
        }
        preconditionFailure("Read \(key) never started")
    }

    @MainActor
    static func operationRegressions() async throws {
        for (delays, expectedCancellation, absent) in [
            (["libraries": Duration.milliseconds(60), "capabilities": .milliseconds(60)], "capabilities", "home"),
            (["item:movie": .milliseconds(60), "credits": .milliseconds(60)], "credits", "children"),
            (["item:movie": .milliseconds(10), "credits": .milliseconds(10), "children": .milliseconds(200)], "children", "unused")
        ] {
            let probe = ReadProbe(delays)
            let store = CatalogStore(client: TimedClient(probe), operationTimeout: .milliseconds(100))
            let bootstrap = expectedCancellation == "capabilities"
            if !bootstrap { store.libraries = [MediaLibrary(id: "movies", title: "Movies")] }
            let started = ContinuousClock.now
            await store.load(screen: "Home", library: "movies", itemID: bootstrap ? nil : "movie")
            let calls = await probe.calls; let cancelled = await probe.cancelled
            precondition(store.error.contains("timed out") && !store.loading && store.detail == nil && store.credits.isEmpty && store.children.isEmpty)
            precondition(cancelled == [expectedCancellation] && !calls.contains(absent))
            precondition(ContinuousClock.now - started < .seconds(1))
            try await Task.sleep(for: .milliseconds(150))
            let finalCalls = await probe.calls
            precondition(store.detail == nil && store.rows.isEmpty && finalCalls == calls)
        }
        let provenanceProbe = ReadProbe(["info": .milliseconds(60), "artwork": .milliseconds(60)])
        let provenance = CatalogStore(client: TimedClient(provenanceProbe), operationTimeout: .milliseconds(100))
        await provenance.loadProvenance()
        let artworkCancellation = await provenanceProbe.cancelled
        precondition(provenance.provenanceError.contains("timed out") && provenance.catalogInfo == nil && provenance.provenanceArtwork.isEmpty && artworkCancellation == ["artwork"])
        let searchProbe = ReadProbe(["search:": .milliseconds(60), "search:last": .milliseconds(100)])
        let search = CatalogStore(client: TimedClient(searchProbe), operationTimeout: .milliseconds(450))
        await search.search(query: "movie", library: "all", options: SearchOptions(source: "all", types: ["Movie"], methods: ["text"]))
        let searchCancellation = await searchProbe.cancelled
        precondition(search.error.contains("timed out") && search.results.isEmpty && searchCancellation == ["search:last"])
        let ancestorProbe = ReadProbe(["item:child": .milliseconds(60), "item:parent": .milliseconds(60)])
        let ancestors = CatalogStore(client: TimedClient(ancestorProbe), operationTimeout: .milliseconds(100))
        do { _ = try await ancestors.ancestors(for: "child"); preconditionFailure("Late ancestors published") }
        catch { precondition(error.localizedDescription.contains("timed out")) }
        let ancestorCancellation = await ancestorProbe.cancelled
        precondition(ancestorCancellation == ["item:parent"])
        let cancelProbe = ReadProbe(["item:child": .seconds(5)])
        let cancelStore = CatalogStore(client: TimedClient(cancelProbe), operationTimeout: .seconds(1))
        let task = Task { try await cancelStore.ancestors(for: "child") }
        await wait(cancelProbe, "item:child")
        task.cancel()
        do { _ = try await task.value; preconditionFailure("Caller cancellation lost") }
        catch { precondition(error is CancellationError) }
        let callerCancellation = await cancelProbe.cancelled
        precondition(callerCancellation == ["item:child"] && cancelStore.error.isEmpty)
        FileHandle.standardOutput.write(Data("Whole-operation monotonic budgets, in-flight cancellation, final-read nonpublication and caller-cancellation checks passed.\n".utf8))
    }

    @MainActor
    static func contextRegressions() async throws {
        let keys: Set<String> = ["library:tv", "home:mock-tmdb", "item:sintel"]
        let probe = ReadProbe(Dictionary(uniqueKeysWithValues: keys.map { ($0, Duration.milliseconds(100)) }), failures: keys)
        let actual = CatalogStore().client!
        let store = CatalogStore(client: ContextClient(actual, probe))
        await store.load(screen: "Library", library: "movies", itemID: nil)
        precondition(store.error.isEmpty && store.items.contains { $0.id == "big-buck-bunny" })
        let tv = Task { await store.load(screen: "Library", library: "tv", itemID: nil) }
        await wait(probe, "library:tv")
        precondition(store.items.isEmpty && store.rows.isEmpty && store.detail == nil && store.loading)
        await tv.value
        precondition(!store.error.isEmpty && store.items.isEmpty)
        await store.load(screen: "Home", library: "all", itemID: nil)
        precondition(store.error.isEmpty && !store.rows.isEmpty)
        store.scope = SourceLibraryScope(sourceId: "mock-tmdb", libraryId: "")
        let home = Task { await store.load(screen: "Home", library: "all", itemID: nil) }
        await wait(probe, "home:mock-tmdb")
        precondition(store.rows.isEmpty && store.items.isEmpty && store.loading)
        await home.value
        precondition(!store.error.isEmpty && store.rows.isEmpty)
        store.scope = SourceLibraryScope(sourceId: "", libraryId: "movies")
        await store.load(screen: "Library", library: "movies", itemID: "big-buck-bunny")
        precondition(store.error.isEmpty && store.detail?.id == "big-buck-bunny" && !store.credits.isEmpty)
        let detail = Task { await store.load(screen: "Library", library: "movies", itemID: "sintel") }
        await wait(probe, "item:sintel")
        precondition(store.detail == nil && store.credits.isEmpty && store.children.isEmpty && store.loading)
        await detail.value
        precondition(!store.error.isEmpty && store.detail == nil && store.credits.isEmpty)
        FileHandle.standardOutput.write(Data("Real-response Movies-to-TV, Home/source and detail context replacement/failure checks passed.\n".utf8))
    }
    static func wait(_ gate: CreditGate, _ id: String) async {
        let deadline = Date().addingTimeInterval(5)
        while Date() < deadline {
            if await gate.pending(id) { return }
            try? await Task.sleep(nanoseconds: 1_000_000)
        }
        preconditionFailure("Delayed request \(id) never reached its async boundary")
    }

    @MainActor
    static func main() async throws {
        try await operationRegressions()
        if ProcessInfo.processInfo.environment["SCREENROOM_STATE_OPERATION_TESTS_ONLY"] == "1" { return }
        try await contextRegressions()
        let missingVotes = MetadataRecord(date: "", rating: Rating(provider: "Example provider", value: 6.1, scale: 10, votes: nil, retrievedAt: ""), providers: [], retrievedAt: "")
        precondition(missingVotes.score == "6.1/10")
        precondition(missingVotes.ratingDetail.contains("Vote count not available"))
        precondition(!missingVotes.ratingDetail.contains("not imported"))
        precondition(MetadataRecord(date: "", rating: Rating(provider: "Other", value: 6.1, scale: 10, votes: 0, retrievedAt: ""), providers: [], retrievedAt: "").score == "Unrated")
        precondition(MetadataRecord(date: "", rating: nil, providers: [], retrievedAt: "").score == "—")
        var variant = testItem("variant")
        variant.colors = ["010203", "aabbcc"]
        variant.rating = Rating(provider: "Independent", value: 3.5, scale: 5, votes: nil, retrievedAt: "rating-date")
        variant.providerReferences = [ProviderReference(provider: "Independent", status: "Available", url: "https://example.com/title"), ProviderReference(provider: "Archive", status: "Unavailable", url: "")]
        variant.provenanceNotes = ["Server note"]
        variant.metadataRetrievedAt = "metadata-date"
        variant.titleRights = Rights(label: "Title rights", note: "Title note", source: "https://example.com/title-rights", licenseUrl: "https://example.com/title-license", attribution: "Title author")
        variant.metadataRights = Rights(label: "Metadata rights", note: "Metadata note", source: "https://example.com/metadata-rights", licenseUrl: "https://example.com/metadata-license", attribution: "Metadata author")
        let mapped = CatalogItem(value: variant)
        precondition(mapped.ink == "010203" && mapped.light == "aabbcc")
        precondition(mapped.record?.score == "3.5/5" && mapped.record?.ratingDetail.contains("Independent") == true)
        precondition(mapped.record?.providers == variant.providerReferences && mapped.record?.retrievedAt == "metadata-date")
        precondition(mapped.value.provenanceNotes == ["Server note"])
        precondition(mapped.rights == "Title rights · Title note" && mapped.attribution == "Title author")
        precondition(mapped.rightsSource == variant.titleRights!.source && mapped.licenseURL == variant.titleRights!.licenseUrl)
        precondition(mapped.value.metadataRights == variant.metadataRights)
        precondition(CatalogItem(value: testItem("neutral")).ink == "303030")
        precondition(externalURL("") == nil && externalURL("relative/path") == nil && externalURL("https://example.com") != nil)
        let caps = SearchCapabilities(methods: ["fuzzy"], types: ["Episode"], sources: ["archive"], semanticDescription: "Server-defined method", maximumQueryLength: 17)
        var options = SearchOptions()
        options.reconcile(caps, preferences: nil)
        precondition(options.types == ["Episode"] && options.methods == ["fuzzy"] && options.source == "archive")
        precondition(!options.titleOnly && options.coreTypes.episode && options.coreMethods.fuzzy)

        let settingsDirectory = URL(fileURLWithPath: #filePath).deletingLastPathComponent().deletingLastPathComponent().appendingPathComponent("build/settings-tests/\(UUID().uuidString)", isDirectory: true)
        let settingsURL = settingsDirectory.appendingPathComponent("roundtrip.json")
        let local = CatalogStore(settingsURL: settingsURL)
        local.scope = SourceLibraryScope(sourceId: "mock-tmdb", libraryId: "")
        let localOptions = SearchOptions(source: "tmdb", types: ["Episode"], methods: ["fuzzy"], titleOnly: true)
        local.save(options: localOptions, library: "tv", subtitle: "library_tracks")
        precondition(local.error.isEmpty, local.error)
        let relaunched = CatalogStore(settingsURL: settingsURL)
        precondition(relaunched.preferences == local.preferences)
        precondition(relaunched.scope == SourceLibraryScope(sourceId: "mock-tmdb", libraryId: "tv") && relaunched.preferences?.source == "tmdb")
        precondition(relaunched.preferences?.types?.episode == true && relaunched.preferences?.types?.movie == false)
        precondition(relaunched.preferences?.methods?.titleOnly == true && relaunched.preferences?.methods?.fuzzy == true && relaunched.preferences?.methods?.semantic == false)
        precondition(relaunched.preferences?.subtitlePreference == "library_tracks")
        let namespace = ProcessInfo.processInfo.environment["SCREENROOM_E2E_SETTINGS_NAMESPACE"] ?? "default"
        let key = "screenroom.settings.v1.\(namespace)"
        let savedData = try Data(contentsOf: settingsURL)
        let savedDocument = try JSONSerialization.jsonObject(with: savedData) as! [String: Any]
        for invalid: [String: Any] in [["version": 2], ["movie": false, "series": false, "episode": false], ["text": false, "fuzzy": false], ["titleOnly": true, "semantic": true]] {
            var document = savedDocument
            document.merge(invalid) { _, value in value }
            let invalidData = try JSONSerialization.data(withJSONObject: document)
            try invalidData.write(to: settingsURL, options: .atomic)
            let invalid = CatalogStore(settingsURL: settingsURL)
            precondition(invalid.preferences == defaultSettings() && !invalid.persistenceWarning.isEmpty)
            let preserved = try Data(contentsOf: settingsURL)
            precondition(preserved == invalidData)
        }
        try Data("invalid".utf8).write(to: settingsURL, options: .atomic)
        precondition(CatalogStore(settingsURL: settingsURL).preferences == defaultSettings())
        let preserved = try Data(contentsOf: settingsURL)
        precondition(preserved == Data("invalid".utf8))
        UserDefaults.standard.set(savedData, forKey: key)
        let migrationURL = settingsDirectory.appendingPathComponent("migration.json")
        let migrated = CatalogStore(settingsURL: migrationURL)
        precondition(migrated.preferences == local.preferences && migrated.persistenceWarning.isEmpty)
        precondition(UserDefaults.standard.data(forKey: key) == savedData)
        UserDefaults.standard.set(Data("invalid legacy".utf8), forKey: key)
        precondition(CatalogStore(settingsURL: migrationURL).preferences == local.preferences)
        UserDefaults.standard.removeObject(forKey: key)
        var failWrite = true
        let failureURL = settingsDirectory.appendingPathComponent("write-recovery.json")
        let failure = CatalogStore(settingsURL: failureURL, settingsWriter: { data, url in
            if failWrite { throw CocoaError(.fileWriteNoPermission) }
            try data.write(to: url, options: .atomic)
        })
        failure.save(options: localOptions, library: "tv", subtitle: "library_tracks")
        precondition(failure.preferences?.source == "tmdb" && !failure.persistenceWarning.isEmpty && failure.error.isEmpty)
        let warning = failure.persistenceWarning
        await failure.load(screen: "Library", library: "tv", itemID: nil)
        precondition(failure.error.isEmpty && failure.persistenceWarning == warning)
        await failure.search(query: "beverly", library: "tv", options: localOptions)
        precondition(failure.error.isEmpty && failure.persistenceWarning == warning)
        _ = try await failure.ancestors(for: "beverly-hillbillies-s1-e1")
        precondition(failure.persistenceWarning == warning)
        failWrite = false
        failure.retrySettingsSave()
        precondition(failure.persistenceWarning.isEmpty && CatalogStore(settingsURL: failureURL).preferences == failure.preferences)
        let mode = try FileManager.default.attributesOfItem(atPath: migrationURL.path)[.posixPermissions] as! NSNumber
        precondition(mode.intValue == 0o600)
        FileHandle.standardOutput.write(Data("Atomic versioned settings roundtrip, migration, corrupt-file preservation and observable write-failure/retry checks passed.\n".utf8))

        let looping = CatalogStore(client: LoopingClient())
        await looping.load(screen: "Home", library: "all", itemID: nil)
        precondition(!looping.error.isEmpty && looping.libraries.isEmpty && !looping.loading)
        await looping.loadProvenance()
        precondition(!looping.provenanceError.isEmpty && looping.provenanceArtwork.isEmpty && !looping.provenanceLoading)

        let gate = CreditGate()
        let store = CatalogStore(client: DelayedClient(gate: gate))
        store.scope = SourceLibraryScope(sourceId: "mock-tmdb", libraryId: "movies")
        store.libraries = [MediaLibrary(id: "all", title: "All libraries")]
        store.libraries.append(MediaLibrary(id: "variant-id", title: "Renamed collection"))
        precondition(store.libraryTitle("variant-id") == "Renamed collection")
        store.detail = CatalogItem(value: testItem("bunny"))
        store.credits = [Credit(id: "bunny", name: "Bunny crew", role: "Director", department: "crew", artwork: nil)]
        store.children = [CatalogItem(value: testItem("bunny-child"))]
        let failing = Task { await store.load(screen: "Library", library: "movies", itemID: "nosferatu") }
        await wait(gate, "nosferatu")
        let requestedScope = await gate.requestedScope("nosferatu")
        precondition(requestedScope == SourceLibraryScope(sourceId: "mock-tmdb", libraryId: "movies"))
        precondition(store.detail == nil && store.credits.isEmpty && store.children.isEmpty)
        await gate.finish("nosferatu", fail: true)
        await failing.value
        precondition(!store.error.isEmpty && store.detail == nil && store.credits.isEmpty && store.children.isEmpty)

        let old = Task { await store.load(screen: "Library", library: "movies", itemID: "old") }
        await wait(gate, "old")
        let latest = Task { await store.load(screen: "Library", library: "movies", itemID: "latest") }
        await wait(gate, "latest")
        await gate.finish("latest")
        await latest.value
        await gate.finish("old")
        await old.value
        precondition(store.detail?.id == "latest" && store.credits.map(\.id) == ["latest"])
        let cancelled = Task { await store.load(screen: "Library", library: "all", itemID: "cancelled") }
        await wait(gate, "cancelled")
        cancelled.cancel()
        await gate.finish("cancelled")
        await cancelled.value
        precondition(store.detail == nil && store.credits.isEmpty && store.children.isEmpty && store.error.isEmpty && !store.loading)
        print("Delayed/failing cross-title state and null-vote regressions passed")

        var baseline: [String] = []
        var snapshotsNonempty = true
        for origin in ["Library", "Search"] {
            let actual = CatalogStore()
            await actual.load(screen: origin, library: "all", itemID: nil)
            precondition(actual.error.isEmpty, actual.error)
            if origin == "Search", let caps = actual.capabilities {
                var options = SearchOptions()
                options.reconcile(caps, preferences: actual.preferences)
                await actual.search(query: "Bunny", library: "all", options: options)
                precondition(actual.error.isEmpty && actual.results.count == 11)
                FileHandle.standardOutput.write(Data("Shared API libraries: \(actual.libraries.map { "\($0.id)=\($0.title)" }); paged hybrid Bunny results: \(actual.results.map(\.id))\n".utf8))
                options.source = "jellyfin"; options.titleOnly = true; options.methods = ["text", "fuzzy"]
                await actual.search(query: "beverly", library: "tv", options: options)
                precondition(actual.error.isEmpty && actual.results.map(\.id) == ["beverly-hillbillies"])
                FileHandle.standardOutput.write(Data("Jellyfin provider-filtered title-only beverly in TV Shows: \(actual.results.map(\.id)); error=\(actual.error)\n".utf8))
            }
            precondition(actual.rows.isEmpty)
            await actual.loadProvenance()
            precondition(actual.provenanceError.isEmpty, actual.provenanceError)
            let arts = actual.provenanceArtwork
            precondition(!arts.isEmpty && actual.provenanceItems.isEmpty)
            let portraitIdentities = [("Sacha Goedegebure", "Sacha Goedegebure"), ("Ton Roosendaal", "Ton Roosendaal"), ("Andreas Goralczyk", "Andy Goralczyk"), ("Jan Morgenstern", "Jan Morgenstern")]
            for (creditName, artworkIdentity) in portraitIdentities {
                precondition(arts.contains { ($0.id == artworkIdentity || $0.label.contains(artworkIdentity)) && !$0.creator.isEmpty && !$0.license.isEmpty && !$0.licenseUrl.isEmpty && !$0.source.isEmpty }, "Missing licensed portrait for \(creditName): \(artworkIdentity)")
            }
            let ids = arts.map(\.id)
            precondition(Set(ids).count == ids.count)
            precondition(arts.count == 21 && actual.catalogInfo?.notices.count == 7)
            let logo = arts.first { $0.kind == "logo" }!
            do {
                _ = try await actual.client!.loadArtwork(url: "http://127.0.0.1:50051/artwork/00000000-0000-0000-0000-000000000000/revisions/1/native")
                preconditionFailure("Missing artwork must not succeed")
            } catch { }
            actual.invalidate()
            let bytes = try await actual.client!.loadArtwork(url: logo.url)
            precondition(bytes.starts(with: [137, 80, 78, 71, 13, 10, 26, 10]) && NSImage(data: bytes) != nil)
            snapshotsNonempty = snapshotsNonempty && !actual.catalogInfo!.retrievedAt.isEmpty
            FileHandle.standardOutput.write(Data("Core CatalogInfo PNG branding request and native decode passed: \(logo.url), \(bytes.count) bytes, \(actual.catalogInfo!.notices.count) ordered notices, snapshot \(actual.catalogInfo!.retrievedAt)\n".utf8))
            if baseline.isEmpty { baseline = ids } else { precondition(baseline == ids) }
            actual.scope = SourceLibraryScope(sourceId: "mock-jellyfin", libraryId: "tv")
            await actual.load(screen: "Library", library: "tv", itemID: "beverly-hillbillies-s1-e1")
            precondition(actual.error.isEmpty, actual.error)
            precondition(actual.detail?.kind == "Episode" && actual.detail?.value.metadataRetrievedAt.isEmpty == false)
            print("On-demand provenance passed from \(origin)-first: \(arts.count) artwork records, no eager title traversal")
        }
        FileHandle.standardOutput.write(Data("All bounded state, local-settings, artwork failure/recovery, complete attribution and scoped episode checks passed.\n".utf8))
        precondition(snapshotsNonempty, "CatalogInfo snapshot timestamp must preserve canonical provenance")
    }
}
