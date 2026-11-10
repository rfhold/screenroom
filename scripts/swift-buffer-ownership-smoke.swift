// Uses real UniFFI RustBuffer allocations and observes calls to their native destructor.
private final class BufferOwnership: @unchecked Sendable {
    private let lock = NSLock()
    private var expected: Set<UInt> = []
    private var freed: [UInt: Int] = [:]

    func expect(_ buffer: RustBuffer) {
        guard buffer.capacity > 0, let pointer = buffer.data else { return }
        lock.lock()
        defer { lock.unlock() }
        expected.insert(UInt(bitPattern: pointer))
    }

    func observe(_ buffer: RustBuffer) {
        guard let pointer = buffer.data else { return }
        lock.lock()
        defer { lock.unlock() }
        let key = UInt(bitPattern: pointer)
        if expected.contains(key) { freed[key, default: 0] += 1 }
    }

    func check(count: Int) throws {
        lock.lock()
        defer { lock.unlock() }
        try require(expected.count == count, "expected \(count) real allocations, observed \(expected.count)")
        try require(expected.allSatisfy { freed[$0] == 1 }, "lowered RustBuffer not destroyed exactly once: \(freed)")
        expected.removeAll()
        freed.removeAll()
    }
}

private let bufferOwnership = BufferOwnership()
fileprivate func screenroomObserveRustBufferFree(_ buffer: RustBuffer) {
    bufferOwnership.observe(buffer)
}

private final class OwnershipGate: @unchecked Sendable {
    private let lock = NSLock()
    private let permit = DispatchSemaphore(value: 0)
    private var arrived = false
    private var completions = 0
    private var frees = 0
    private var status: Int8 = -1

    func block() {
        lock.lock()
        arrived = true
        lock.unlock()
        permit.wait()
    }
    func isBlocked() -> Bool {
        lock.lock()
        defer { lock.unlock() }
        return arrived
    }
    func release() { permit.signal() }
    func completed(_ returned: RustBuffer, _ status: RustCallStatus) {
        lock.lock()
        completions += 1
        self.status = status.code
        lock.unlock()
        bufferOwnership.expect(returned)
        bufferOwnership.expect(status.errorBuf)
    }
    func freed() {
        lock.lock()
        frees += 1
        lock.unlock()
    }
    func check(status: Int8) throws {
        lock.lock()
        defer { lock.unlock() }
        try require(completions == 1 && frees == 1 && self.status == status, "complete/free/status mismatch: \(completions)/\(frees)/\(self.status)")
    }
}

private final class OwnershipPoll: @unchecked Sendable {
    let gate: OwnershipGate
    let callback: UniffiRustFutureContinuationCallback
    let data: UInt64
    init(_ gate: OwnershipGate, _ callback: @escaping UniffiRustFutureContinuationCallback, _ data: UInt64) {
        self.gate = gate
        self.callback = callback
        self.data = data
    }
}
private let ownershipPolls = UniffiHandleMap<OwnershipPoll>()

func runBufferOwnershipRegression(endpoint: String) async throws {
    for outcome in ["success", "error", "before-poll"] {
        smokeProgress("RustBuffer ownership \(outcome)")
        let gate = OwnershipGate()
        let client = try Client(config: ClientConfig(endpoint: endpoint, userId: testUserID("swift-owner-\(outcome)"), timeoutMs: 5000))
        let task = Task.detached {
            try await uniffiRustCallAsync(
                rustFutureFunc: {
                    let future = uniffi_screenroom_client_core_fn_method_client_get_item(
                        client.uniffiCloneHandle(), FfiConverterString.lower(outcome == "error" ? "missing" : "transport-value"),
                        FfiConverterTypeSourceLibraryScope.lower(SourceLibraryScope(sourceId: "", libraryId: ""))
                    )
                    if outcome == "before-poll" { gate.block() }
                    return future
                },
                pollFunc: { future, callback, data in
                    ffi_screenroom_client_core_rust_future_poll_rust_buffer(future, { context, result in
                        let poll = try! ownershipPolls.remove(handle: context)
                        // The actual pinned UniFFI future has lowered and stored its RustBuffer here.
                        if result == UNIFFI_RUST_FUTURE_POLL_READY { poll.gate.block() }
                        poll.callback(poll.data, result)
                    }, ownershipPolls.insert(obj: OwnershipPoll(gate, callback, data)))
                },
                completeFunc: { future, status in
                    let returned = ffi_screenroom_client_core_rust_future_complete_rust_buffer(future, status)
                    gate.completed(returned, status.pointee)
                    return returned
                },
                cancelFunc: ffi_screenroom_client_core_rust_future_cancel_rust_buffer,
                freeFunc: { future in
                    ffi_screenroom_client_core_rust_future_free_rust_buffer(future)
                    gate.freed()
                },
                liftFunc: FfiConverterTypeItem.lift,
                errorHandler: FfiConverterTypeClientError_lift
            )
        }
        while !gate.isBlocked() { try await Task.sleep(nanoseconds: 100_000) }
        task.cancel()
        gate.release()
        try await expectCancellation(task)
        try gate.check(status: outcome == "success" ? CALL_SUCCESS : outcome == "error" ? CALL_ERROR : CALL_CANCELLED)
        try bufferOwnership.check(count: outcome == "before-poll" ? 0 : 1)
    }

    // A panic status also carries an owned Rust allocation. Do not send it through the fatal cancelled-status translator.
    let panic = OwnershipGate()
    let task = Task.detached {
        try await uniffiRustCallAsync(
            rustFutureFunc: { panic.block(); return 1 },
            pollFunc: { _, _, _ in preconditionFailure("cancelled before poll") },
            completeFunc: { _, status in
                status.pointee.code = CALL_UNEXPECTED_ERROR
                status.pointee.errorBuf = FfiConverterString.lower("nonempty panic allocation")
                let returned = RustBuffer.empty()
                panic.completed(returned, status.pointee)
                return returned
            },
            cancelFunc: { _ in }, freeFunc: { _ in panic.freed() },
            liftFunc: FfiConverterString.lift, errorHandler: nil
        )
    }
    while !panic.isBlocked() { try await Task.sleep(nanoseconds: 100_000) }
    task.cancel()
    panic.release()
    try await expectCancellation(task)
    try panic.check(status: CALL_UNEXPECTED_ERROR)
    try bufferOwnership.check(count: 1)
    print("RustBuffer ownership PASS: actual lowered success/error after Ready cancellation, before-initial-poll cancellation, allocated panic status; every owned allocation destroyed exactly once")
}
