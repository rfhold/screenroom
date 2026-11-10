#!/usr/bin/env python3
"""Apply the checked UniFFI 0.31.2 per-future Swift cancellation correction."""
import re
import sys
from pathlib import Path

ORIGINAL = '''fileprivate func uniffiRustCallAsync<F, T>(
    rustFutureFunc: () -> UInt64,
    pollFunc: (UInt64, @escaping UniffiRustFutureContinuationCallback, UInt64) -> (),
    completeFunc: (UInt64, UnsafeMutablePointer<RustCallStatus>) -> F,
    freeFunc: (UInt64) -> (),
    liftFunc: (F) throws -> T,
    errorHandler: ((RustBuffer) throws -> Swift.Error)?
) async throws -> T {
    // Make sure to call the ensure init function since future creation doesn't have a
    // RustCallStatus param, so doesn't use makeRustCall()
    uniffiEnsureScreenroomClientCoreInitialized()
    let rustFuture = rustFutureFunc()
    defer {
        freeFunc(rustFuture)
    }
    var pollResult: Int8;
    repeat {
        pollResult = await withUnsafeContinuation {
            pollFunc(
                rustFuture,
                { handle, pollResult in
                    uniffiFutureContinuationCallback(handle: handle, pollResult: pollResult)
                },
                uniffiContinuationHandleMap.insert(obj: $0)
            )
        }
    } while pollResult != UNIFFI_RUST_FUTURE_POLL_READY

    return try liftFunc(makeRustCall(
        { completeFunc(rustFuture, $0) },
        errorHandler: errorHandler
    ))
}'''

REPLACEMENT = '''// Checked correction for UniFFI 0.31.2: Swift cancellation owns only this future.
fileprivate final class ScreenroomRustFutureCancellation: @unchecked Sendable {
    private let lock = NSLock()
    private var handle: UInt64?
    private var cancelled = false
    private var completed = false
    private let cancelFunc: (UInt64) -> Void
    private let freeFunc: (UInt64) -> Void

    init(cancelFunc: @escaping (UInt64) -> Void, freeFunc: @escaping (UInt64) -> Void) {
        self.cancelFunc = cancelFunc
        self.freeFunc = freeFunc
    }

    func install(_ handle: UInt64) {
        lock.lock()
        defer { lock.unlock() }
        self.handle = handle
        if cancelled { cancelFunc(handle) }
    }

    func cancel() {
        lock.lock()
        defer { lock.unlock() }
        cancelled = true
        if let handle = handle { cancelFunc(handle) }
    }

    func checkCancellation() throws {
        lock.lock()
        defer { lock.unlock() }
        if cancelled { throw CancellationError() }
    }

    func complete<T>(_ operation: () throws -> T) throws -> T {
        lock.lock()
        defer { lock.unlock() }
        // Never translate UniFFI's cancelled RustCallStatus through makeRustCall.
        if cancelled { throw CancellationError() }
        completed = true
        return try operation()
    }

    func free(discardCompletion: (UInt64) -> Void) {
        lock.lock()
        defer { lock.unlock() }
        if let handle = handle {
            self.handle = nil
            if !completed {
                completed = true
                discardCompletion(handle)
            }
            freeFunc(handle)
        }
    }
}

fileprivate func uniffiRustCallAsync<T>(
    rustFutureFunc: () -> UInt64,
    pollFunc: (UInt64, @escaping UniffiRustFutureContinuationCallback, UInt64) -> (),
    completeFunc: (UInt64, UnsafeMutablePointer<RustCallStatus>) -> RustBuffer,
    cancelFunc: @escaping (UInt64) -> (),
    freeFunc: @escaping (UInt64) -> (),
    liftFunc: (RustBuffer) throws -> T,
    errorHandler: ((RustBuffer) throws -> Swift.Error)?
) async throws -> T {
    uniffiEnsureScreenroomClientCoreInitialized()
    let cancellation = ScreenroomRustFutureCancellation(cancelFunc: cancelFunc, freeFunc: freeFunc)
    return try await withTaskCancellationHandler(operation: {
        try Task.checkCancellation()
        let rustFuture = rustFutureFunc()
        cancellation.install(rustFuture)
        defer {
            cancellation.free { handle in
                // UniFFI free() drops raw RustBuffers without reclaiming their allocations.
                var status = RustCallStatus()
                let returned = completeFunc(handle, &status)
                returned.deallocate()
                status.errorBuf.deallocate()
            }
        }
        var pollResult: Int8
        repeat {
            try cancellation.checkCancellation()
            pollResult = await withUnsafeContinuation {
                pollFunc(
                    rustFuture,
                    { handle, pollResult in
                        // Rust holds its scheduler lock; resuming here can invert Swift's cancellation lock.
                        DispatchQueue.global().async {
                            uniffiFutureContinuationCallback(handle: handle, pollResult: pollResult)
                        }
                    },
                    uniffiContinuationHandleMap.insert(obj: $0)
                )
            }
        } while pollResult != UNIFFI_RUST_FUTURE_POLL_READY
        return try cancellation.complete {
            try liftFunc(makeRustCall(
                { completeFunc(rustFuture, $0) },
                errorHandler: errorHandler
            ))
        }
    }, onCancel: { cancellation.cancel() })
}'''


def patch(source, header):
    if source.count(ORIGINAL) != 1:
        raise ValueError("expected exactly one unmodified UniFFI 0.31.2 async bridge; inspect generator changes")
    pattern = re.compile(r"(?m)^( +)completeFunc: ffi_screenroom_client_core_rust_future_complete_(\w+),$")
    calls = list(pattern.finditer(source))
    if len(calls) != 12 or source.count("await uniffiRustCallAsync(") != 12:
        raise ValueError("expected exactly 12 async Client calls; inspect generator/API changes")
    for match in calls:
        if match[2] != "rust_buffer":
            raise ValueError("Swift cancellation ownership correction only supports RustBuffer futures")
        symbol = f"ffi_screenroom_client_core_rust_future_cancel_{match[2]}"
        if len(re.findall(r"\b" + symbol + r"\s*\(", header)) != 1:
            raise ValueError(f"missing or ambiguous exported cancellation symbol: {symbol}")
    source = source.replace(ORIGINAL, REPLACEMENT)
    return pattern.sub(lambda m: f"{m[0]}\n{m[1]}cancelFunc: ffi_screenroom_client_core_rust_future_cancel_{m[2]},", source)


if __name__ == "__main__":
    directory = Path(sys.argv[1])
    swift = directory / "ScreenroomClientCore.swift"
    try:
        updated = patch(swift.read_text(), (directory / "ScreenroomClientCoreFFI.h").read_text())
    except ValueError as error:
        sys.exit(f"Swift cancellation correction refused: {error}")
    swift.write_text(updated)
