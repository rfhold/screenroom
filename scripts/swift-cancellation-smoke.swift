import Foundation

enum SmokeFailure: Error { case failed(String) }

func smokeProgress(_ message: String) {
    FileHandle.standardError.write(Data((message + "\n").utf8))
}

func require(_ condition: @autoclosure () -> Bool, _ message: String) throws {
    if !condition() { throw SmokeFailure.failed(message) }
}

func expectCancellation<T>(_ task: Task<T, Error>) async throws {
    do {
        _ = try await task.value
        throw SmokeFailure.failed("cancelled task completed successfully")
    } catch is CancellationError {
    } catch ClientError.Cancelled {
    }
}

actor StartGate {
    private var continuation: CheckedContinuation<Void, Never>?
    private var opened = false
    func wait() async {
        if opened { return }
        await withCheckedContinuation { continuation = $0 }
    }
    func open() {
        opened = true
        continuation?.resume()
        continuation = nil
    }
}

@main struct CancellationSmoke {
    static func main() async throws {
        try await runLifecycleRegression()
        smokeProgress("native pre-cancel")
        let endpoint = ProcessInfo.processInfo.environment["SCREENROOM_SWIFT_TEST_ENDPOINT"]!
        try await runBufferOwnershipRegression(endpoint: endpoint)
        func client(_ label: String) throws -> Client {
            try Client(config: ClientConfig(endpoint: endpoint, userId: testUserID(label), timeoutMs: 5000))
        }

        smokeProgress("native transport info + independent cancellation")
        let catalog = try client("swift-catalog-info")
        let cancelledInfo = Task { try await catalog.getCatalogInfo() }
        let survivingInfo = Task { try await catalog.getCatalogInfo() }
        try await Task.sleep(nanoseconds: 150_000_000)
        cancelledInfo.cancel()
        try await expectCancellation(cancelledInfo)
        let info = try await survivingInfo.value
        try require(info.branding.isEmpty && info.notices.isEmpty, "transport double response corrupt")

        let pre = try client("swift-pre-cancel")
        for _ in 0..<50 {
            let gate = StartGate()
            let task = Task { await gate.wait(); return try await pre.getItem(id: "transport-value", scope: SourceLibraryScope(sourceId: "", libraryId: "")) }
            task.cancel()
            await gate.open()
            try await expectCancellation(task)
        }

        let shared = try client("swift-dedup")
        smokeProgress("native dedup")
        let first = Task { try await shared.getItem(id: "transport-value", scope: SourceLibraryScope(sourceId: "", libraryId: "")) }
        let survivor = Task { try await shared.getItem(id: "transport-value", scope: SourceLibraryScope(sourceId: "", libraryId: "")) }
        let independent = Task { try await shared.listLibraries(sourceId: "", page: Page(size: 50, token: "")) }
        try await Task.sleep(nanoseconds: 150_000_000)
        let start = ContinuousClock.now
        first.cancel()
        try await expectCancellation(first)
        try require(start.duration(to: .now) < .milliseconds(400), "individual cancellation was not prompt")
        let survivingItem = try await survivor.value
        let libraries = try await independent.value
        try require(survivingItem.title == "transport-value", "deduplicated survivor failed")
        try require(libraries.libraries.isEmpty, "independent request failed")

        let capacity = try client("swift-capacity")
        smokeProgress("native capacity")
        let cancelled = (0..<32).map { n in Task { try await capacity.getItem(id: "cancelled-\(n)", scope: SourceLibraryScope(sourceId: "", libraryId: "")) } }
        try await Task.sleep(nanoseconds: 150_000_000)
        let capacityStart = ContinuousClock.now
        for task in cancelled { task.cancel() }
        for task in cancelled { try await expectCancellation(task) }
        try require(capacityStart.duration(to: .now) < .milliseconds(400), "32 cancellations waited for responses")
        // All 32 Rust pending slots must be reusable BEFORE the 800ms server delay / 5s deadline.
        let retries = (0..<32).map { n in
            Task {
                do {
                    _ = try await capacity.getItem(id: "replacement-\(n)", scope: SourceLibraryScope(sourceId: "", libraryId: ""))
                    throw SmokeFailure.failed("missing replacement item unexpectedly exists")
                } catch ClientError.NotFound { return }
            }
        }
        for task in retries { try await task.value }

        var racedCancellations = 0
        smokeProgress("native completion races")
        var racedCompletions = 0
        for n in 0..<100 {
            if n % 10 == 0 { smokeProgress("native completion race \(n)") }
            let racing = try client("swift-race-\(n)")
            let task = Task { try await racing.getHome(scope: SourceLibraryScope(sourceId: "", libraryId: "")) }
            try await Task.sleep(nanoseconds: UInt64(n % 12) * 1_000_000)
            task.cancel()
            do {
                let home = try await task.value
                try require(home.isEmpty, "racing response corrupt")
                racedCompletions += 1
            } catch is CancellationError {
                racedCancellations += 1
            } catch ClientError.Cancelled {
                racedCancellations += 1
            }
        }
        try require(racedCancellations > 0 && racedCompletions > 0, "race did not cover both outcomes")
        print("Swift cancellation PASS: 50 pre-cancel, dedup survivor, independent read, 32 slots immediately reused, 100 completion races (\(racedCancellations) cancelled / \(racedCompletions) complete)")
    }
}

// Deterministic trusted-local test selectors, not authentication.
func testUserID(_ label: String) -> String {
    String(format: "00000000-0000-0000-0000-%012d", label.utf8.reduce(0) { $0 + Int($1) })
}
