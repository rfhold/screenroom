import Foundation
import AppKit

private func testItem(_ id: String) -> Item {
    Item(id: id, title: id, kind: "Movie", parentId: nil, year: 2000, subtitle: "", overview: "", tags: [], colors: [], releaseDate: "", addedOrder: 0, titleRights: nil, metadataRights: nil, artwork: nil, rating: nil, runtime: nil, providerReferences: [], progressPercent: nil, provenanceNotes: [], metadataRetrievedAt: "")
}

private actor CreditGate {
    private var requests: [String: CheckedContinuation<CreditPage, Error>] = [:]
    func read(_ id: String) async throws -> CreditPage {
        try await withCheckedThrowingContinuation { requests[id] = $0 }
    }
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
    override func getItem(id: String) async throws -> Item { testItem(id) }
    override func listCredits(itemId: String, page: Page) async throws -> CreditPage { try await gate.read(itemId) }
    override func listChildren(parentId: String, page: Page) async throws -> ItemPage { ItemPage(items: [], nextPageToken: "") }
}

@main
private struct CatalogStateTests {
    static func wait(_ gate: CreditGate, _ id: String) async {
        for _ in 0..<10000 {
            if await gate.pending(id) { return }
            await Task.yield()
        }
        preconditionFailure("Delayed request never reached its async boundary")
    }

    @MainActor
    static func main() async throws {
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

        let gate = CreditGate()
        let store = CatalogStore(client: DelayedClient(gate: gate))
        store.libraries = [MediaLibrary(id: "all", title: "All libraries")]
        store.libraries.append(MediaLibrary(id: "variant-id", title: "Renamed collection"))
        precondition(store.libraryTitle("variant-id") == "Renamed collection")
        store.detail = CatalogItem(value: testItem("bunny"))
        store.credits = [Credit(id: "bunny", name: "Bunny crew", role: "Director", department: "crew", artwork: nil)]
        store.children = [CatalogItem(value: testItem("bunny-child"))]
        let failing = Task { await store.load(screen: "Library", library: "All libraries", itemID: "nosferatu") }
        await wait(gate, "nosferatu")
        precondition(store.detail == nil && store.credits.isEmpty && store.children.isEmpty)
        await gate.finish("nosferatu", fail: true)
        await failing.value
        precondition(!store.error.isEmpty && store.detail == nil && store.credits.isEmpty && store.children.isEmpty)

        let old = Task { await store.load(screen: "Library", library: "All libraries", itemID: "old") }
        await wait(gate, "old")
        let latest = Task { await store.load(screen: "Library", library: "All libraries", itemID: "latest") }
        await wait(gate, "latest")
        await gate.finish("latest")
        await latest.value
        await gate.finish("old")
        await old.value
        precondition(store.detail?.id == "latest" && store.credits.map(\.id) == ["latest"])
        print("Delayed/failing cross-title state and null-vote regressions passed")

        var baseline: [String] = []
        for origin in ["Library", "Search"] {
            let actual = CatalogStore()
            await actual.load(screen: origin, library: "all", itemID: nil)
            precondition(actual.error.isEmpty, actual.error)
            if origin == "Search", let caps = actual.capabilities {
                var options = SearchOptions()
                options.reconcile(caps, preferences: actual.preferences)
                await actual.search(query: "Bunny", library: "all", options: options)
            }
            precondition(actual.rows.isEmpty)
            await actual.loadProvenance()
            precondition(actual.provenanceError.isEmpty, actual.provenanceError)
            let arts = actual.provenanceArtwork
            precondition(!arts.isEmpty && !actual.provenanceItems.isEmpty)
            let portraitIdentities = [("Sacha Goedegebure", "Sacha Goedegebure"), ("Ton Roosendaal", "Ton Roosendaal"), ("Andreas Goralczyk", "Andy Goralczyk"), ("Jan Morgenstern", "Jan Morgenstern")]
            for (creditName, artworkIdentity) in portraitIdentities {
                precondition(arts.contains { ($0.id == artworkIdentity || $0.label.contains(artworkIdentity)) && !$0.creator.isEmpty && !$0.license.isEmpty && !$0.licenseUrl.isEmpty && !$0.source.isEmpty }, "Missing licensed portrait for \(creditName): \(artworkIdentity)")
            }
            let ids = arts.map(\.id)
            precondition(Set(ids).count == ids.count)
            precondition(arts.count == 11 && actual.catalogInfo?.notices.count == 7)
            let logo = arts.first { $0.kind == "logo" }!
            let bytes = try await actual.client!.loadArtwork(url: logo.url)
            precondition(bytes.starts(with: [137, 80, 78, 71, 13, 10, 26, 10]) && NSImage(data: bytes) != nil)
            precondition(!actual.catalogInfo!.retrievedAt.isEmpty)
            precondition(actual.provenanceItems.contains { !$0.value.provenanceNotes.isEmpty })
            precondition(actual.provenanceItems.allSatisfy { !$0.value.metadataRetrievedAt.isEmpty })
            print("Core CatalogInfo PNG branding request and native decode passed: \(logo.url), \(bytes.count) bytes, \(actual.catalogInfo!.notices.count) ordered notices")
            if baseline.isEmpty { baseline = ids } else { precondition(baseline == ids) }
            precondition(actual.provenanceItems.contains { $0.kind == "Episode" })
            print("Core provenance passed from \(origin)-first: \(actual.provenanceItems.count) titles, \(arts.count) artwork records")
        }
    }
}
