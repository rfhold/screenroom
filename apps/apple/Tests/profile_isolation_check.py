"""Run extracted Swift profile logic without bindings, GUI, or network."""

from pathlib import Path
import subprocess


apple = Path(__file__).resolve().parents[1]
source = (apple / "Sources/Home/Catalog.swift").read_text()
views = (apple / "Sources/Home/MetadataViews.swift").read_text()
tests = (apple / "Tests/CatalogStateTests.swift").read_text()

assert "let reads = try Client(config: context)" in source
assert "let profiles = try Client(config: context)" in source
assert "profileClient = profiles" in source
assert "profileClient !== client" in source
assert "profile.load(client: catalog.profileClient" in views
assert "profile.change(client: catalog.profileClient" in views

store = source[source.index("@MainActor @Observable\nfinal class JellyfinProfileStore"):
               source.index("struct SearchOptions:")]
budget = source[source.index("private struct OperationTimeout:"):
                source.index("private struct SettingsFile {")]
fakes = tests[tests.index("private final class InvalidationProbe"):
              tests.index("private final class ProfileClient:")]
home_start = source.index('            } else if screen == "Home" {')
home_body = source[home_start + len('            } else if screen == "Home" {'):
                   source.index('            } else if screen == "Library" {', home_start)]
home = r'''
@MainActor final class HomeReadHarness {
    let client: Client
    let jellyfinProfile: JellyfinProfileStore
    var generation = 0
    var homeProfileRevision = 0
    var rows: [HomeRow] = []
    init(client: Client, profile: JellyfinProfileStore) { self.client = client; jellyfinProfile = profile }
    func load() async throws {
        let client = self.client
        let budget = OperationBudget(.seconds(5))
        generation += 1
        let current = generation
        let requestedScope = SourceLibraryScope(sourceId: "", libraryId: "")
''' + home_body + "\n    }\n}\n"

stubs = r'''
import Foundation
import Observation
struct Page: Sendable { var size: Int; var token: String }
struct JellyfinSource: Sendable { var id: String; var displayName: String }
struct JellyfinProfile: Sendable { var id: String; var displayName: String }
struct JellyfinLink: Sendable { var sourceId: String; var jellyfinUserId: String }
struct JellyfinSourcePage: Sendable { var sources: [JellyfinSource]; var nextPageToken: String }
struct JellyfinProfilePage: Sendable { var profiles: [JellyfinProfile]; var nextPageToken: String }
struct SourceLibraryScope: Sendable { var sourceId: String; var libraryId: String }
struct HomeRow: Sendable {}
class Client: @unchecked Sendable {
    struct NoHandle {}
    init(noHandle: NoHandle) {}
    required init(unsafeFromHandle: UInt64) {}
    func invalidateReads() {}
    func listJellyfinSources(page: Page) async throws -> JellyfinSourcePage { fatalError() }
    func listJellyfinProfiles(sourceId: String, page: Page) async throws -> JellyfinProfilePage { fatalError() }
    func getJellyfinLink(sourceId: String) async throws -> JellyfinLink? { fatalError() }
    func setJellyfinLink(sourceId: String, jellyfinUserId: String) async throws -> JellyfinLink { fatalError() }
    func clearJellyfinLink(sourceId: String) async throws -> String { fatalError() }
    func getHome(scope: SourceLibraryScope) async throws -> [HomeRow] { fatalError() }
}
'''

checks = r'''
let check = Task { @MainActor in
    for cancel in [false, true] {
        let profiles = DelayedProfileClient()
        let reads = DelayedProfileClient(probe: profiles.probe)
        let store = JellyfinProfileStore()
        let homeStore = HomeReadHarness(client: reads, profile: store)
        await store.load(client: profiles)
        let old = Task { await store.change(client: profiles, profile: "profile") }
        let deadline = ContinuousClock.now.advanced(by: .seconds(5))
        while !(await profiles.probe.pending) && ContinuousClock.now < deadline { await Task.yield() }
        let pending = await profiles.probe.pending
        precondition(pending)
        store.deactivate()
        if cancel { old.cancel() }
        let before = profiles.invalidations.count
        await store.load(client: profiles)
        precondition(!store.known && store.error.contains("still settling"))
        store.deactivate()
        let home = Task { try await homeStore.load() }
        while !(await profiles.probe.homeReadStarted) && ContinuousClock.now < deadline { await Task.yield() }
        let homeStarted = await profiles.probe.homeReadStarted
        precondition(homeStarted)
        await profiles.probe.finish()
        await old.value
        _ = try await home.value
        precondition(profiles.invalidations.count == before + 1 && reads.invalidations.count == 0)
        precondition(!store.known && !store.busy && store.revision == 0 && store.link == nil)
        await store.load(client: profiles)
        let writes = await profiles.probe.writes
        precondition(store.known && store.currentProfile == "Viewer" && store.revision == 1 && writes == 1)
        try await homeStore.load()
        precondition(reads.invalidations.count == 1)
        try await homeStore.load()
        precondition(reads.invalidations.count == 1)
    }
    print("PASS profile isolation: automatic settlement invalidation, active Home survives, generation/cancellation, unknown rapid re-entry, one-write read-only reconciliation")
}
try await check.value
'''

subprocess.run(["swift", "-swift-version", "6", "-"],
               input=stubs + budget + store + fakes + home + checks, text=True, check=True)
