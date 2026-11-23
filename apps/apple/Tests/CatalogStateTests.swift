import Foundation
import AppKit

private func testItem(_ id: String) -> Item {
    Item(id: id, title: id, kind: "Movie", parentId: nil, year: 2000, subtitle: "", overview: "", tags: [], colors: [], releaseDate: "", addedOrder: 0, titleRights: nil, metadataRights: nil, artwork: nil, rating: nil, runtime: nil, providerReferences: [], progressPercent: nil, provenanceNotes: [], metadataRetrievedAt: "", watched: nil)
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
    override func listLibraryItems(scope: SourceLibraryScope, page: Page) async throws -> ItemPage {
        ItemPage(items: [], nextPageToken: "")
    }
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

private actor ProfileProbe {
    var writes = 0
    var mapping: JellyfinLink?
    var failRead = false
    func mutate(_ profile: String?) throws {
        writes += 1
        mapping = profile.map { JellyfinLink(sourceId: "source", jellyfinUserId: $0) }
        failRead = true
        throw CocoaError(.fileReadUnknown)
    }
    func read() throws -> JellyfinLink? {
        if failRead { failRead = false; throw CocoaError(.fileReadUnknown) }
        return mapping
    }
}

private final class InvalidationProbe: @unchecked Sendable {
    private let lock = NSLock()
    private var value = 0
    func record() { lock.lock(); defer { lock.unlock() }; value += 1 }
    var count: Int { lock.lock(); defer { lock.unlock() }; return value }
}

private actor DelayedWriteProbe {
    var writes = 0
    var homeReadStarted = false
    var mapping: JellyfinLink?
    private var continuation: CheckedContinuation<Void, Never>?
    var pending: Bool { continuation != nil }
    func save() async -> JellyfinLink {
        writes += 1
        await withCheckedContinuation { continuation = $0 }
        let saved = JellyfinLink(sourceId: "source", jellyfinUserId: "profile")
        mapping = saved
        return saved
    }
    func finish() { continuation?.resume(); continuation = nil }
    func startHome() { homeReadStarted = true }
}

private final class DelayedProfileClient: Client, @unchecked Sendable {
    let probe: DelayedWriteProbe
    let invalidations = InvalidationProbe()
    init(probe: DelayedWriteProbe = DelayedWriteProbe()) { self.probe = probe; super.init(noHandle: NoHandle()) }
    required init(unsafeFromHandle handle: UInt64) { fatalError("Test-only transport") }
    override func invalidateReads() { invalidations.record() }
    override func listJellyfinSources(page: Page) async throws -> JellyfinSourcePage {
        JellyfinSourcePage(sources: [JellyfinSource(id: "source", displayName: "Server")], nextPageToken: "")
    }
    override func listJellyfinProfiles(sourceId: String, page: Page) async throws -> JellyfinProfilePage {
        JellyfinProfilePage(profiles: [JellyfinProfile(id: "profile", displayName: "Viewer")], nextPageToken: "")
    }
    override func getJellyfinLink(sourceId: String) async throws -> JellyfinLink? { await probe.mapping }
    override func setJellyfinLink(sourceId: String, jellyfinUserId: String) async throws -> JellyfinLink {
        defer { invalidateReads() }
        return await probe.save()
    }
    override func getHome(scope: SourceLibraryScope) async throws -> [HomeRow] {
        let before = invalidations.count
        await probe.startHome()
        try await Task.sleep(for: .milliseconds(50))
        precondition(invalidations.count == before, "Departed profile operation invalidated current Home reads")
        return []
    }
}

private final class ProfileClient: Client, @unchecked Sendable {
    let probe = ProfileProbe()
    init() { super.init(noHandle: NoHandle()) }
    required init(unsafeFromHandle handle: UInt64) { fatalError("Test-only transport") }
    override func invalidateReads() {}
    override func listJellyfinSources(page: Page) async throws -> JellyfinSourcePage {
        JellyfinSourcePage(sources: page.token.isEmpty ? [JellyfinSource(id: "source", displayName: "Server")] : [JellyfinSource(id: "source", displayName: "Server"), JellyfinSource(id: "other", displayName: "Other server")], nextPageToken: page.token.isEmpty ? "last" : "")
    }
    override func listJellyfinProfiles(sourceId: String, page: Page) async throws -> JellyfinProfilePage {
        JellyfinProfilePage(profiles: page.token.isEmpty ? [JellyfinProfile(id: "profile", displayName: "Viewer")] : [JellyfinProfile(id: "profile", displayName: "Viewer"), JellyfinProfile(id: "other", displayName: "Other viewer")], nextPageToken: page.token.isEmpty ? "last" : "")
    }
    override func getJellyfinLink(sourceId: String) async throws -> JellyfinLink? { try await probe.read() }
    override func setJellyfinLink(sourceId: String, jellyfinUserId: String) async throws -> JellyfinLink {
        try await probe.mutate(jellyfinUserId)
        throw CocoaError(.fileReadUnknown)
    }
    override func clearJellyfinLink(sourceId: String) async throws -> String {
        try await probe.mutate(nil)
        return sourceId
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

private final class BoundaryClient: Client, @unchecked Sendable {
    init() { super.init(noHandle: NoHandle()) }
    required init(unsafeFromHandle handle: UInt64) { fatalError("Test-only transport") }
    override func listLibraries(sourceId: String, page: Page) async throws -> LibraryPage {
        LibraryPage(libraries: [MediaLibrary(id: "movies", title: "Movies"), MediaLibrary(id: "tv", title: "TV")], nextPageToken: "")
    }
    override func getSearchCapabilities() async throws -> SearchCapabilities {
        SearchCapabilities(methods: ["text"], types: ["Movie"], sources: ["all"], semanticDescription: "", maximumQueryLength: 100)
    }
    override func listLibraryItems(scope: SourceLibraryScope, page: Page) async throws -> ItemPage {
        ItemPage(items: [testItem("boundary-item")], nextPageToken: "")
    }
    override func getHome(scope: SourceLibraryScope) async throws -> [HomeRow] {
        [HomeRow(id: "boundary-row", title: "Boundary row", items: [testItem("boundary-item")], readError: nil)]
    }
    override func getItem(id: String, scope: SourceLibraryScope) async throws -> Item { testItem(id) }
    override func listCredits(itemId: String, page: Page, scope: SourceLibraryScope) async throws -> CreditPage {
        let art = Artwork(id: "portrait", url: "https://example.test/portrait.png", label: "Portrait", creator: "Creator", license: "License", source: "https://example.test/source", licenseUrl: "https://example.test/license", kind: "portrait")
        return CreditPage(credits: [Credit(id: "credit", name: "Credit", role: "Role", department: "cast", artwork: art)], nextPageToken: "")
    }
    override func listChildren(parentId: String, page: Page, scope: SourceLibraryScope) async throws -> ItemPage { ItemPage(items: [], nextPageToken: "") }
}

private actor NavigationProbe {
    private(set) var requests: [String] = []
    private var held: CheckedContinuation<ItemPage, Error>?
    var waiting: Bool { held != nil }
    func page(scope: SourceLibraryScope, token: String, search: SearchRequest? = nil) async throws -> ItemPage {
        let prefix = search == nil ? "library" : "search"
        requests.append("\(prefix)|\(scope.sourceId)|\(scope.libraryId)|\(token)|\(search?.query ?? "")")
        if scope.libraryId == "slow" {
            return try await withCheckedThrowingContinuation { held = $0 }
        }
        if token == "third" { throw CocoaError(.fileReadUnknown) }
        return ItemPage(items: [testItem("\(prefix)-\(token.isEmpty ? "1" : "2")")], nextPageToken: token.isEmpty ? "second" : "third")
    }
    func release() {
        held?.resume(returning: ItemPage(items: [testItem("stale")], nextPageToken: ""))
        held = nil
    }
}

private final class NavigationClient: Client, @unchecked Sendable {
    let probe: NavigationProbe
    init(_ probe: NavigationProbe) { self.probe = probe; super.init(noHandle: NoHandle()) }
    required init(unsafeFromHandle handle: UInt64) { fatalError("Test-only transport") }
    override func invalidateReads() {}
    override func listLibraryItems(scope: SourceLibraryScope, page: Page) async throws -> ItemPage {
        try await probe.page(scope: scope, token: page.token)
    }
    override func search(request: SearchRequest) async throws -> SearchPage {
        let values = try await probe.page(scope: SourceLibraryScope(sourceId: request.sourceId, libraryId: request.libraryId),
            token: request.page?.token ?? "", search: request)
        return SearchPage(items: values.items, nextPageToken: values.nextPageToken, effectiveMethods: request.methods, degradationReason: "test-degradation")
    }
    override func getItem(id: String, scope: SourceLibraryScope) async throws -> Item { testItem(id) }
    override func listCredits(itemId: String, page: Page, scope: SourceLibraryScope) async throws -> CreditPage {
        CreditPage(credits: [], nextPageToken: "")
    }
    override func listChildren(parentId: String, page: Page, scope: SourceLibraryScope) async throws -> ItemPage {
        ItemPage(items: [], nextPageToken: "")
    }
}

@main
private struct CatalogStateTests {
    @MainActor
    static func navigationRegressions() async throws {
        let probe = NavigationProbe()
        let store = CatalogStore(client: NavigationClient(probe), pageSize: 1)
        store.scope = SourceLibraryScope(sourceId: "source-a", libraryId: "")
        store.libraries = [MediaLibrary(id: "movies", title: "Movies")]
        let options = SearchOptions(source: "all", types: ["Movie"], methods: ["text"], titleOnly: false)
        func key(_ screen: String = "Library", _ item: String = "", source: String = "source-a", library: String = "movies") -> CatalogNavigationKey {
            CatalogNavigationKey(screen: screen, library: library, itemID: item, query: screen == "Search" ? "query" : "",
                source: "all", types: ["Movie"], methods: ["text"], titleOnly: false, sourceID: source)
        }
        let library = key()
        await store.visit(library, options: options)
        await store.loadMore()
        await store.loadMore()
        let libraryError = store.pageError
        precondition(store.items.map(\.id) == ["library-1", "library-2"] && !libraryError.isEmpty)
        store.navigationMemory.remember(library, anchor: "library-2", focus: "card.library-2")
        let beforeDetail = await probe.requests
        await store.visit(key("Library", "library-2"), options: options)
        await store.visit(library, options: options)
        let afterLibraryBack = await probe.requests
        precondition(afterLibraryBack == beforeDetail, "Back must not fetch and replace page one")
        precondition(store.items.map(\.id) == ["library-1", "library-2"])
        precondition(store.itemPage.token == "third" && store.itemPage.pages == 2 && store.pageError == libraryError)
        precondition(store.navigationMemory.anchor(for: library) == "library-2" && store.navigationMemory.focus(for: library) == "card.library-2")
        await store.loadMore()
        let resumedLibrary = await probe.requests
        precondition(resumedLibrary.last == "library|source-a|movies|third|")

        let search = key("Search")
        await store.visit(search, options: options)
        await store.loadMore(search: true)
        await store.loadMore(search: true)
        let searchError = store.pageError
        precondition(store.results.map(\.id) == ["search-1", "search-2"] && !searchError.isEmpty)
        store.navigationMemory.remember(search, anchor: "search-2", focus: "card.search-2")
        let searchBeforeDetail = await probe.requests
        await store.visit(key("Search", "search-2"), options: options)
        await store.visit(search, options: options)
        let searchAfterBack = await probe.requests
        precondition(searchAfterBack == searchBeforeDetail, "Search Back must not replace the retained page prefix")
        precondition(store.results.map(\.id) == ["search-1", "search-2"])
        precondition(store.searchPage.token == "third" && store.searchPage.pages == 2 && store.pageError == searchError)
        precondition(store.searchDegradation == "test-degradation" && store.effectiveSearchMethods == options.coreMethods)
        precondition(store.navigationMemory.anchor(for: search) == "search-2" && store.navigationMemory.focus(for: search) == "card.search-2")
        await store.loadMore(search: true)
        let resumedSearch = await probe.requests
        precondition(resumedSearch.last == "search|source-a|movies|third|query")

        var alternateOptions = options
        alternateOptions.methods = ["fuzzy"]
        let alternate = CatalogNavigationKey(screen: "Search", library: "movies", itemID: "", query: "other query",
            source: "all", types: ["Movie"], methods: ["fuzzy"], titleOnly: false, sourceID: "source-a")
        await store.visit(alternate, options: alternateOptions)
        let alternateRequests = await probe.requests
        precondition(alternateRequests.last == "search|source-a|movies||other query" && store.results.map(\.id) == ["search-1"])
        let alternateCount = alternateRequests.count
        await store.visit(search, options: options)
        let restoredOriginalCount = await probe.requests.count
        precondition(restoredOriginalCount == alternateCount && store.results.map(\.id) == ["search-1", "search-2"])
        precondition(store.searchPage.token == "third" && store.pageError == searchError)

        store.jellyfinProfile.revision += 1
        await store.visit(search, options: options)
        precondition(store.results.map(\.id) == ["search-1"] && store.searchPage.token == "second")
        precondition(store.navigationMemory.anchor(for: search) == nil, "Profile change must discard retained positions and results")
        store.scope = SourceLibraryScope(sourceId: "source-b", libraryId: "")
        let rejected = await store.visit(library, options: options)
        precondition(!rejected && store.results.isEmpty && store.items.isEmpty)
        store.scope = SourceLibraryScope(sourceId: "source-a", libraryId: "")
        store.libraries = [MediaLibrary(id: "movies", title: "Movies")]
        await store.visit(library, options: options)
        precondition(store.items.map(\.id) == ["library-1"], "A previous source generation cannot restore page two")

        let stale = Task { await store.visit(key(library: "slow"), options: options) }
        let deadline = ContinuousClock.now.advanced(by: .seconds(5))
        while !(await probe.waiting) {
            precondition(ContinuousClock.now < deadline)
            await Task.yield()
        }
        store.jellyfinProfile.revision += 1
        await store.visit(library, options: options)
        await probe.release()
        let staleAccepted = await stale.value
        precondition(!staleAccepted && store.items.map(\.id) == ["library-1"])

        let otherClient = CatalogStore(client: NavigationClient(probe), pageSize: 1)
        otherClient.scope = SourceLibraryScope(sourceId: "source-a", libraryId: "")
        otherClient.libraries = [MediaLibrary(id: "movies", title: "Movies")]
        let beforeNewClient = await probe.requests.count
        await otherClient.visit(library, options: options)
        let afterNewClient = await probe.requests.count
        precondition(afterNewClient == beforeNewClient + 1 && otherClient.items.map(\.id) == ["library-1"])
        await otherClient.loadMore()
        otherClient.navigationMemory.remember(library, anchor: "library-2", focus: "card.library-2")
        for index in 0..<17 {
            await otherClient.visit(key("Library", "detail-\(index)"), options: options)
        }
        let beforeEvictedEntry = await probe.requests.count
        await otherClient.visit(library, options: options)
        let afterEvictedEntry = await probe.requests.count
        precondition(afterEvictedEntry == beforeEvictedEntry + 1 && otherClient.items.map(\.id) == ["library-1"])
        precondition(otherClient.navigationMemory.anchor(for: library) == nil, "Entry eviction must also drop its scroll/focus targets")
        print("Entry-owned two-page library/search restoration, continuation errors, position/focus targets, profile/source/client fences and stale-response rejection passed")
    }
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
        let providerProbe = ReadProbe([:])
        let provider = CatalogStore(client: TimedClient(providerProbe))
        await provider.load(screen: "Home", library: "all", itemID: nil)
        let providerCalls = await providerProbe.calls
        precondition(provider.error.isEmpty && !providerCalls.contains("info"))
        let searchProbe = ReadProbe(["search:": .milliseconds(60), "search:last": .milliseconds(600)])
        let search = CatalogStore(client: TimedClient(searchProbe), operationTimeout: .milliseconds(450))
        await search.search(query: "movie", library: "all", options: SearchOptions(source: "all", types: ["Movie"], methods: ["text"]))
        precondition(search.error.isEmpty && search.results.count == 1 && search.searchPage.token == "last")
        await search.loadMore(search: true)
        let searchCancellation = await searchProbe.cancelled
        precondition(!search.pageError.isEmpty && search.results.count == 1 && search.searchPage.token == "last" && searchCancellation == ["search:last"])
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
    static func parityRegressions() async throws {
        var item = testItem("episode")
        item.kind = "Episode"; item.title = "Episode title"
        var season = testItem("season")
        season.kind = "Season"
        item.parentId = "season"
        precondition(CatalogItem(value: item).isSeasonChild(of: CatalogItem(value: season)))
        item.parentId = "different-season"
        precondition(!CatalogItem(value: item).isSeasonChild(of: CatalogItem(value: season)))
        item.parentId = nil
        precondition(!CatalogItem(value: item).isSeasonChild(of: CatalogItem(value: season)))
        item.parentId = "season"; item.kind = "Movie"
        precondition(!CatalogItem(value: item).isSeasonChild(of: CatalogItem(value: season)))
        item.kind = "Episode"; season.kind = "Series"
        precondition(!CatalogItem(value: item).isSeasonChild(of: CatalogItem(value: season)))
        precondition(!CatalogItem(value: item).isSeasonChild(of: nil))
        for (subtitle, heading, season) in [
            ("Series · S05E09", "Series · S05:E09", "E09"),
            ("Series: Part · Two · S5 E9 · 40 min", "Series: Part · Two · S05:E09", "E09"),
            ("Series · S0E100", "Series · S00:E100", "E100"),
            ("S100E0", "S100:E00", "E00"),
            ("Series · missing", "Episode", "Episode")
        ] {
            item.subtitle = subtitle
            let value = CatalogItem(value: item)
            precondition(value.labels().heading == heading)
            precondition(value.labels(seasonContext: true).heading == season)
            precondition(value.labels().subtitle == item.title)
        }
        precondition(!homeRowError("progress_unavailable").contains("private"))
        precondition(homeRowError("private id and server error") == "This row could not be loaded. Other catalog rows remain available.")
        var page = ItemPagination()
        var values = try page.append((0..<100).map { testItem(String($0)) }, next: "next", to: [])
        precondition(values.count == 100 && loadedItemCount(values.count) == "100 items loaded")
        values = try page.append([testItem("99"), testItem("101"), testItem("100")], next: "final", to: values)
        precondition(values.suffix(2).map(\.id) == ["101", "100"])
        precondition(loadedItemCount(values.count) == "102 items loaded" && loadedItemCount(1) == "1 item loaded")
        do { _ = try page.append([testItem("bad")], next: "next", to: values); preconditionFailure("Token cycle accepted") }
        catch { precondition(page.token == "final" && values.count == 102 && loadedItemCount(values.count) == "102 items loaded") }

        let delayedClient = DelayedProfileClient()
        let readClient = DelayedProfileClient(probe: delayedClient.probe)
        let activeHome = CatalogStore(client: readClient, profileClient: delayedClient)
        precondition(activeHome.client !== activeHome.profileClient)
        activeHome.libraries = [MediaLibrary(id: "all", title: "All libraries")]
        let departedProfile = activeHome.jellyfinProfile
        await departedProfile.load(client: delayedClient)
        let write = Task { await departedProfile.change(client: delayedClient, profile: "profile") }
        let deadline = ContinuousClock.now.advanced(by: .seconds(5))
        while !(await delayedClient.probe.pending) && ContinuousClock.now < deadline { await Task.yield() }
        let pending = await delayedClient.probe.pending
        precondition(pending)
        departedProfile.deactivate(); write.cancel()
        let invalidationsBefore = delayedClient.invalidations.count
        await departedProfile.load(client: delayedClient)
        precondition(!departedProfile.known && departedProfile.error.contains("still settling") && delayedClient.invalidations.count == invalidationsBefore)
        departedProfile.deactivate()
        let home = Task { await activeHome.load(screen: "Home", library: "all", itemID: nil) }
        let homeDeadline = ContinuousClock.now.advanced(by: .seconds(5))
        while !(await delayedClient.probe.homeReadStarted) && ContinuousClock.now < homeDeadline { await Task.yield() }
        let homeStarted = await delayedClient.probe.homeReadStarted
        precondition(homeStarted)
        await delayedClient.probe.finish()
        await write.value
        await home.value
        precondition(delayedClient.invalidations.count == invalidationsBefore + 1 && readClient.invalidations.count == 0)
        precondition(activeHome.error.isEmpty && !activeHome.loading)
        precondition(!departedProfile.known && !departedProfile.busy && departedProfile.link == nil && departedProfile.revision == 0 && departedProfile.error.contains("still settling"))
        await departedProfile.load(client: delayedClient)
        let departedWrites = await delayedClient.probe.writes
        precondition(departedWrites == 1 && departedProfile.currentProfile == "Viewer" && departedProfile.revision == 1)
        await activeHome.load(screen: "Home", library: "all", itemID: nil)
        precondition(activeHome.error.isEmpty && readClient.invalidations.count == 1)
        await activeHome.load(screen: "Home", library: "all", itemID: nil)
        precondition(activeHome.error.isEmpty && readClient.invalidations.count == 1)

        let client = ProfileClient()
        let profile = JellyfinProfileStore()
        await profile.load(client: client)
        precondition(profile.known && profile.currentProfile == "Not linked" && profile.sources.count == 2 && profile.profiles.count == 2)
        await profile.change(client: client, profile: "profile")
        precondition(!profile.known && profile.error.contains("will not be repeated"))
        await profile.change(client: client, profile: "profile")
        profile.deactivate()
        await profile.load(client: client)
        let writes = await client.probe.writes
        precondition(writes == 1 && profile.known && profile.currentProfile == "Viewer" && profile.revision == 1)
        profile.link = JellyfinLink(sourceId: "source", jellyfinUserId: "unavailable")
        precondition(profile.currentProfile == "Linked profile unavailable in choices")

        let paginationProbe = ReadProbe(["search:last": .milliseconds(200)])
        let search = CatalogStore(client: TimedClient(paginationProbe), operationTimeout: .seconds(2))
        let options = SearchOptions(source: "all", types: ["Movie"], methods: ["text"])
        await search.search(query: "first", library: "all", options: options)
        let oldMore = Task { await search.loadMore(search: true) }
        await wait(paginationProbe, "search:last")
        await search.loadMore(search: true)
        await search.search(query: "new query", library: "movies", options: options)
        await oldMore.value
        let continuationCalls = await paginationProbe.calls.filter { $0 == "search:last" }
        precondition(continuationCalls.count == 1 && search.results.count == 1 && search.searchPage.token == "last")

        let loader = ArtworkLoader()
        let probe = ReadProbe(Dictionary(uniqueKeysWithValues: (0..<16).map { ("art:\($0)", Duration.milliseconds(100)) }))
        let tasks = (0..<16).map { n in Task { try await loader.load(key: "\(n)") { try await probe.pause("art:\(n)"); return Data([UInt8(n)]) } } }
        await wait(probe, "art:7")
        let firstCalls = await probe.calls
        precondition(firstCalls.count == 8)
        tasks[0].cancel(); tasks[8].cancel()
        for task in tasks { _ = try? await task.value }
        precondition(loader.peakActive == 8)
        let finalCalls = await probe.calls
        precondition(!finalCalls.contains("art:8"))
        let a = Task { try await loader.load(key: "shared") { try await probe.pause("shared"); return Data([1]) } }
        let b = Task { try await loader.load(key: "shared") { try await probe.pause("shared"); return Data([1]) } }
        _ = try await a.value; _ = try await b.value
        let sharedCalls = await probe.calls.filter { $0 == "shared" }
        precondition(sharedCalls.count == 1)
    }

    @MainActor
    static func contextRegressions() async throws {
        let keys: Set<String> = ["library:tv", "home:source-id", "item:detail-id"]
        let probe = ReadProbe(Dictionary(uniqueKeysWithValues: keys.map { ($0, Duration.milliseconds(100)) }), failures: keys)
        let actual = BoundaryClient()
        let store = CatalogStore(client: ContextClient(actual, probe))
        await store.load(screen: "Library", library: "movies", itemID: nil)
        precondition(store.error.isEmpty && store.items.contains { $0.id == "boundary-item" })
        let tv = Task { await store.load(screen: "Library", library: "tv", itemID: nil) }
        await wait(probe, "library:tv")
        precondition(store.items.isEmpty && store.rows.isEmpty && store.detail == nil && store.loading)
        await tv.value
        precondition(!store.error.isEmpty && store.items.isEmpty)
        await store.load(screen: "Home", library: "all", itemID: nil)
        precondition(store.error.isEmpty && !store.rows.isEmpty)
        store.scope = SourceLibraryScope(sourceId: "source-id", libraryId: "")
        let home = Task { await store.load(screen: "Home", library: "all", itemID: nil) }
        await wait(probe, "home:source-id")
        precondition(store.rows.isEmpty && store.items.isEmpty && store.loading)
        await home.value
        precondition(!store.error.isEmpty && store.rows.isEmpty)
        store.scope = SourceLibraryScope(sourceId: "", libraryId: "movies")
        await store.load(screen: "Library", library: "movies", itemID: "boundary-item")
        precondition(store.error.isEmpty && store.detail?.id == "boundary-item" && !store.credits.isEmpty)
        let portraits = store.credits.compactMap(\.artwork).filter { $0.kind == "portrait" }
        precondition(!portraits.isEmpty && portraits.allSatisfy { !$0.creator.isEmpty && !$0.license.isEmpty && !$0.licenseUrl.isEmpty && !$0.source.isEmpty })
        let detail = Task { await store.load(screen: "Library", library: "movies", itemID: "detail-id") }
        await wait(probe, "item:detail-id")
        precondition(store.detail == nil && store.credits.isEmpty && store.children.isEmpty && store.loading)
        await detail.value
        precondition(!store.error.isEmpty && store.detail == nil && store.credits.isEmpty)
        print("Isolated library, Home/source and detail context replacement/failure checks passed")
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
        try await navigationRegressions()
        var watchedItem = testItem("watched-state")
        watchedItem.progressPercent = 100
        for state in [nil, false, true, false, nil, true] as [Bool?] {
            watchedItem.watched = state
            let presentation = CatalogItem(value: watchedItem)
            precondition(presentation.watched == (state == true))
            precondition(presentation.cardAccessibility.components(separatedBy: "Watched").count == (state == true ? 2 : 1))
            precondition(presentation.playbackAccessibility.contains("Playback progress 100%"))
            precondition(presentation.id == "watched-state")
        }
        watchedItem.progressPercent = nil
        precondition(CatalogItem(value: watchedItem).playbackAccessibility == "Watched")
        try await parityRegressions()
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
        variant.artwork = Artwork(id: "poster", url: "https://example.com/poster.png", label: "Poster", creator: "Poster artist", license: "Poster CC BY", source: "https://example.com/poster-source", licenseUrl: "https://example.com/poster-license", kind: "poster")
        let mapped = CatalogItem(value: variant)
        precondition(mapped.ink == "010203" && mapped.light == "aabbcc")
        precondition(mapped.record?.score == "3.5/5" && mapped.record?.ratingDetail.contains("Independent") == true)
        precondition(mapped.record?.providers == variant.providerReferences && mapped.record?.retrievedAt == "metadata-date")
        precondition(mapped.value.provenanceNotes == ["Server note"])
        precondition(mapped.rights == "Title rights · Title note" && mapped.attribution == "Title author")
        precondition(mapped.rightsSource == variant.titleRights!.source && mapped.licenseURL == variant.titleRights!.licenseUrl)
        precondition(mapped.value.metadataRights == variant.metadataRights)
        precondition(mapped.artwork == variant.artwork && mapped.artwork?.creator == "Poster artist")
        precondition(mapped.artwork?.creator != mapped.attribution && mapped.artwork?.licenseUrl != mapped.licenseURL)
        precondition(mapped.artwork?.source == "https://example.com/poster-source")
        precondition(CatalogItem(value: testItem("neutral")).ink == "303030")
        precondition(externalURL("") == nil && externalURL("relative/path") == nil && externalURL("https://example.com") != nil)
        precondition(externalURL("https://user:secret@example.com") == nil)
        precondition(externalURL("javascript:alert(1)") == nil)
        precondition(providerStatus("source_supplied") == "Identifier supplied by Jellyfin")
        precondition(MetadataRecord(date: "", rating: Rating(provider: "Invalid", value: .nan, scale: 0, votes: nil, retrievedAt: ""), providers: [], retrievedAt: "").score == "—")
        var navigation = CatalogNavigationMemory()
        func key(_ library: String, _ item: String = "", _ query: String = "query") -> CatalogNavigationKey {
            CatalogNavigationKey(screen: "Search", library: library, itemID: item, query: query, source: "all",
                types: ["Movie"], methods: ["text"], titleOnly: false)
        }
        let origin = key("library")
        navigation.remember(origin, anchor: "movie", focus: "card.movie")
        navigation.remember(key("library", "movie"), anchor: "cast")
        precondition(navigation.anchor(for: origin) == "movie" && navigation.focus(for: origin) == "card.movie")
        precondition(navigation.anchor(for: key("another-library")) == nil && navigation.anchor(for: key("library", "", "other-query")) == nil)
        for index in 0..<65 { navigation.remember(key("library-\(index)"), anchor: "row") }
        precondition(navigation.anchor(for: origin) == nil, "Navigation restoration must be bounded")
        let caps = SearchCapabilities(methods: ["fuzzy"], types: ["Episode"], sources: ["archive"], semanticDescription: "Server-defined method", maximumQueryLength: 17)
        var options = SearchOptions()
        options.reconcile(caps, preferences: nil)
        precondition(options.types == ["Episode"] && options.methods == ["fuzzy"] && options.source == "archive")
        precondition(!options.titleOnly && options.coreTypes.episode && options.coreMethods.fuzzy)

        let settingsDirectory = URL(fileURLWithPath: #filePath).deletingLastPathComponent().deletingLastPathComponent().appendingPathComponent("build/settings-tests/\(UUID().uuidString)", isDirectory: true)
        let settingsURL = settingsDirectory.appendingPathComponent("roundtrip.json")
        let local = CatalogStore(settingsURL: settingsURL)
        local.scope = SourceLibraryScope(sourceId: "source-id", libraryId: "")
        let localOptions = SearchOptions(source: "jellyfin", types: ["Episode"], methods: ["fuzzy"], titleOnly: true)
        local.save(options: localOptions, library: "tv")
        precondition(local.error.isEmpty, local.error)
        let relaunched = CatalogStore(settingsURL: settingsURL)
        precondition(relaunched.preferences == local.preferences)
        precondition(relaunched.scope == SourceLibraryScope(sourceId: "source-id", libraryId: "tv") && relaunched.preferences?.source == "jellyfin")
        precondition(relaunched.preferences?.types?.episode == true && relaunched.preferences?.types?.movie == false)
        precondition(relaunched.preferences?.methods?.titleOnly == true && relaunched.preferences?.methods?.fuzzy == true && relaunched.preferences?.methods?.semantic == false)
        let namespace = ProcessInfo.processInfo.environment["SCREENROOM_E2E_SETTINGS_NAMESPACE"] ?? "default"
        let key = "screenroom.settings.v1.\(namespace)"
        let savedData = try Data(contentsOf: settingsURL)
        let savedDocument = try JSONSerialization.jsonObject(with: savedData) as! [String: Any]
        precondition(savedDocument["subtitle"] == nil)
        var unsupportedOptions = localOptions
        unsupportedOptions.source = "tmdb"
        local.save(options: unsupportedOptions, library: "tv")
        precondition(local.preferences == relaunched.preferences && !local.persistenceWarning.isEmpty)
        let afterUnsupportedSave = try Data(contentsOf: settingsURL)
        precondition(afterUnsupportedSave == savedData)
        for subtitle in [nil, "transcription"] as [String?] {
            var legacy = savedDocument
            legacy["source"] = "tmdb"
            legacy["subtitle"] = subtitle
            try JSONSerialization.data(withJSONObject: legacy).write(to: settingsURL, options: .atomic)
            var expected = local.preferences!
            expected.source = "all"
            precondition(CatalogStore(settingsURL: settingsURL).preferences == expected)
        }
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
        let failure = CatalogStore(client: TimedClient(ReadProbe([:])), settingsURL: failureURL, settingsWriter: { data, url in
            if failWrite { throw CocoaError(.fileWriteNoPermission) }
            try data.write(to: url, options: .atomic)
        })
        failure.save(options: localOptions, library: "tv")
        precondition(failure.preferences?.source == "jellyfin" && !failure.persistenceWarning.isEmpty && failure.error.isEmpty)
        let warning = failure.persistenceWarning
        await failure.load(screen: "Library", library: "tv", itemID: nil)
        precondition(failure.error.isEmpty && failure.persistenceWarning == warning)
        await failure.search(query: "query", library: "tv", options: localOptions)
        precondition(failure.error.isEmpty && failure.persistenceWarning == warning)
        _ = try await failure.ancestors(for: "episode-id")
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

        let gate = CreditGate()
        let store = CatalogStore(client: DelayedClient(gate: gate))
        store.scope = SourceLibraryScope(sourceId: "source-id", libraryId: "movies")
        store.libraries = [MediaLibrary(id: "all", title: "All libraries")]
        store.libraries.append(MediaLibrary(id: "variant-id", title: "Renamed collection"))
        precondition(store.libraryTitle("variant-id") == "Renamed collection")
        store.detail = CatalogItem(value: testItem("prior-item"))
        store.credits = [Credit(id: "prior-credit", name: "Prior credit", role: "Director", department: "crew", artwork: nil)]
        store.children = [CatalogItem(value: testItem("prior-child"))]
        let failing = Task { await store.load(screen: "Library", library: "movies", itemID: "failed-item") }
        await wait(gate, "failed-item")
        let requestedScope = await gate.requestedScope("failed-item")
        precondition(requestedScope == SourceLibraryScope(sourceId: "source-id", libraryId: "movies"))
        precondition(store.detail == nil && store.credits.isEmpty && store.children.isEmpty)
        await gate.finish("failed-item", fail: true)
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

        if ProcessInfo.processInfo.environment["SCREENROOM_STATE_LOCAL_TESTS_ONLY"] == "1" {
            print("Local state checks passed; real-source integration was not requested")
            return
        }
        for origin in ["Library", "Search"] {
            let actual = CatalogStore()
            await actual.load(screen: origin, library: "all", itemID: nil)
            precondition(actual.error.isEmpty, actual.error)
            precondition(actual.rows.isEmpty)
            let libraries = actual.libraries.filter { $0.id != "all" }
            for library in libraries {
                await actual.load(screen: "Library", library: library.id, itemID: nil)
                precondition(actual.error.isEmpty, actual.error)
                for _ in 0..<512 {
                    if actual.itemPage.token.isEmpty { break }
                    await actual.loadMore(search: false)
                    precondition(actual.pageError.isEmpty, actual.pageError)
                }
                precondition(actual.itemPage.token.isEmpty, "Library exceeded bounded test traversal")
                precondition(Set(actual.items.map(\.id)).count == actual.items.count)
                if let item = actual.items.first {
                    await actual.load(screen: "Library", library: library.id, itemID: item.id)
                    precondition(actual.error.isEmpty && actual.detail?.id == item.id, actual.error)
                    if let art = item.value.artwork {
                        let bytes = try await actual.client!.loadArtwork(url: art.url)
                        precondition(NSImage(data: bytes) != nil, "Published artwork must decode natively")
                    }
                    if origin == "Search", let caps = actual.capabilities {
                        var options = SearchOptions()
                        options.reconcile(caps, preferences: actual.preferences)
                        options.titleOnly = true
                        await actual.search(query: item.title, library: library.id, options: options)
                        for _ in 0..<512 {
                            if actual.searchPage.token.isEmpty { break }
                            await actual.loadMore(search: true)
                            precondition(actual.pageError.isEmpty, actual.pageError)
                        }
                        precondition(actual.error.isEmpty && actual.searchPage.token.isEmpty, actual.error)
                        precondition(actual.results.contains { $0.id == item.id }, "Title search must retain the discovered item")
                    }
                }
            }
            print("\(origin)-first source-driven catalog: \(libraries.count) libraries")
        }
        print("Bounded state, local settings and source-driven catalog checks passed")
    }
}
