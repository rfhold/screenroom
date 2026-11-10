// Concatenated with generated Swift so the private bridge can be exercised deterministically.
private final class LifecycleFuture: @unchecked Sendable {
    private let lock = NSLock()
    private var callback: (UniffiRustFutureContinuationCallback, UInt64)?
    private var cancelled = false
    private var created = false
    private var counts = (cancel: 0, free: 0, complete: 0, poll: 0, invalid: 0)
    let createDelay: UInt32
    let wakeDelay: Int
    private let holdSchedulerDuringCallback: Bool
    private let schedulerHeld = DispatchSemaphore(value: 0)
    private let cancelEntered = DispatchSemaphore(value: 0)

    init(createDelay: UInt32 = 0, wakeDelay: Int = 100, holdSchedulerDuringCallback: Bool = false) {
        self.createDelay = createDelay
        self.wakeDelay = wakeDelay
        self.holdSchedulerDuringCallback = holdSchedulerDuringCallback
    }

    func waitForScheduler() -> Bool {
        schedulerHeld.wait(timeout: .now() + 5) == .success
    }

    func snapshot() -> (created: Bool, cancel: Int, free: Int, complete: Int, poll: Int, invalid: Int) {
        lock.lock()
        defer { lock.unlock() }
        return (created, counts.cancel, counts.free, counts.complete, counts.poll, counts.invalid)
    }

    private func create() -> UInt64 {
        lock.lock()
        created = true
        lock.unlock()
        if createDelay > 0 { usleep(createDelay) }
        return 7
    }

    private func poll(_ handle: UInt64, _ callback: @escaping UniffiRustFutureContinuationCallback, _ data: UInt64) {
        lock.lock()
        counts.poll += 1
        if counts.free != 0 || self.callback != nil { counts.invalid += 1 }
        let ready = cancelled
        if !ready { self.callback = (callback, data) }
        lock.unlock()
        if ready { callback(data, 0) }
        else { DispatchQueue.global().asyncAfter(deadline: .now() + .microseconds(wakeDelay)) { self.wake() } }
    }

    private func wake() {
        lock.lock()
        let pending = callback
        callback = nil
        if holdSchedulerDuringCallback, let (callback, data) = pending {
            schedulerHeld.signal()
            precondition(cancelEntered.wait(timeout: .now() + 5) == .success, "cancellation never entered scheduler")
            smokeProgress("bridge scheduler callback under lock")
            callback(data, 0)
            smokeProgress("bridge scheduler callback returned")
            lock.unlock()
            return
        }
        lock.unlock()
        if let (callback, data) = pending { callback(data, 0) }
    }

    private func cancel(_ handle: UInt64) {
        if holdSchedulerDuringCallback {
            smokeProgress("bridge scheduler cancellation entering held lock")
            cancelEntered.signal()
        }
        lock.lock()
        counts.cancel += 1
        if counts.free != 0 { counts.invalid += 1 }
        cancelled = true
        lock.unlock()
        wake()
    }

    private func complete(_ handle: UInt64, _ status: UnsafeMutablePointer<RustCallStatus>) -> RustBuffer {
        lock.lock()
        defer { lock.unlock() }
        counts.complete += 1
        if counts.free != 0 { counts.invalid += 1 }
        if cancelled {
            status.pointee.code = CALL_CANCELLED
            return .empty()
        }
        return FfiConverterString.lower(String(handle))
    }

    private func free(_ handle: UInt64) {
        lock.lock()
        counts.free += 1
        if counts.free != 1 || counts.complete != 1 || callback != nil { counts.invalid += 1 }
        lock.unlock()
    }

    func run() async throws -> String {
        try await uniffiRustCallAsync(
            rustFutureFunc: create, pollFunc: poll, completeFunc: complete,
            cancelFunc: cancel, freeFunc: free, liftFunc: FfiConverterString.lift, errorHandler: nil
        )
    }
}

func runSchedulerLockRegression() async throws {
    smokeProgress("bridge scheduler lock inversion")
    let future = LifecycleFuture(holdSchedulerDuringCallback: true)
    let task = Task.detached { try await future.run() }
    let held = await Task.detached { future.waitForScheduler() }.value
    try require(held, "scheduler never held callback lock")
    task.cancel()
    try await expectCancellation(task)
    let s = future.snapshot()
    try require(s.cancel == 1 && s.free == 1 && s.complete == 1 && s.invalid == 0, "scheduler cancellation lifecycle invalid: \(s)")
    smokeProgress("bridge scheduler lock inversion PASS")
}

func runLifecycleRegression() async throws {
    try await runSchedulerLockRegression()
    for phase in ["create", "poll"] {
        smokeProgress("bridge \(phase)")
        let future = LifecycleFuture(createDelay: phase == "create" ? 50_000 : 0, wakeDelay: 1_000_000)
        let task = Task.detached { try await future.run() }
        while phase == "create" ? !future.snapshot().created : future.snapshot().poll == 0 {
            try await Task.sleep(nanoseconds: 100_000)
        }
        task.cancel()
        try await expectCancellation(task)
        let s = future.snapshot()
        try require(s.cancel == 1 && s.free == 1 && s.complete == 1 && s.invalid == 0, "\(phase) cancellation lifecycle invalid: \(s)")
    }
    let completed = LifecycleFuture(wakeDelay: 0)
    smokeProgress("bridge post-free")
    let task = Task.detached { try await completed.run() }
    let result = try await task.value
    task.cancel()
    let s = completed.snapshot()
    try require(result == "7" && s.cancel == 0 && s.free == 1 && s.complete == 1 && s.invalid == 0, "post-free cancellation called FFI")

    for n in 0..<1000 {
        if n % 100 == 0 { smokeProgress("bridge race \(n)") }
        let future = LifecycleFuture()
        let task = Task.detached { try await future.run() }
        while future.snapshot().poll == 0 { await Task.yield() }
        if n % 3 != 0 { try await Task.sleep(nanoseconds: UInt64(n % 3) * 100_000) }
        task.cancel()
        do { _ = try await task.value } catch is CancellationError { }
        let s = future.snapshot()
        try require(s.free == 1 && s.complete == 1 && s.cancel <= 1 && s.invalid == 0, "racing lifecycle invalid: \(s)")
    }
    print("Swift bridge lifecycle PASS: cancellation during create/poll, cancellation after free, 1000 exactly-once completion/free races")
}
