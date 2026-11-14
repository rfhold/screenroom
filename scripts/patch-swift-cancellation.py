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
    generated = source.replace(ORIGINAL, "")
    methods = re.compile(
        r"^(?:open|public) func [^\n]*\basync\b[^\n]*\{\n(?P<body>.*?)^\}",
        re.MULTILINE | re.DOTALL,
    )
    call = re.compile(
        r"    return\n"
        r"        try {1,2}await uniffiRustCallAsync\(\n"
        r"            rustFutureFunc: \{\n"
        r"                uniffi_screenroom_client_core_fn_method_[a-z0-9_]+\(\n"
        r"(?: {20}[^\n]*\n)+"
        r"                \)\n"
        r"            \},\n"
        r"            pollFunc: ffi_screenroom_client_core_rust_future_poll_(?P<poll>\w+),\n"
        r"            completeFunc: ffi_screenroom_client_core_rust_future_complete_(?P<complete>\w+),\n"
        r"            freeFunc: ffi_screenroom_client_core_rust_future_free_(?P<free>\w+),\n"
        r"            liftFunc: FfiConverter\w+(?:\.lift|_lift),\n"
        r"            errorHandler: (?:FfiConverter\w+_lift|nil)\n"
        r"        \)\n"
    )
    future_reference = re.compile(r"\b(?:uniffiRustCallAsync|ffi_screenroom_client_core_rust_future_\w+)\b")
    found = list(methods.finditer(generated))
    declarations = re.findall(r"(?m)^[^\n]*\bfunc\b[^\n]*\basync\b[^\n]*\{[ \t]*$", generated)
    if not found or len(found) != len(declarations):
        raise ValueError("missing or unmatched generated async methods; inspect generator/API changes")
    for method in found:
        body = method["body"]
        match = call.fullmatch(body)
        if match is None:
            raise ValueError("unrecognized generated async callsite; inspect generator/API changes")
        suffixes = [match[name] for name in ("poll", "complete", "free")]
        expected = ["uniffiRustCallAsync"] + [
            f"ffi_screenroom_client_core_rust_future_{operation}_{suffix}"
            for operation, suffix in zip(("poll", "complete", "free"), suffixes)
        ]
        if future_reference.findall(body) != expected:
            raise ValueError("unmatched async or future references; inspect generator/API changes")
        if any(suffix != "rust_buffer" for suffix in suffixes):
            raise ValueError("Swift cancellation ownership correction only supports RustBuffer futures")
    if future_reference.search(methods.sub("", generated)):
        raise ValueError("unmatched async or future references; inspect generator/API changes")
    symbol = "ffi_screenroom_client_core_rust_future_cancel_rust_buffer"
    if len(re.findall(r"\b" + symbol + r"\s*\(", header)) != 1:
        raise ValueError(f"missing or ambiguous exported cancellation symbol: {symbol}")
    if not re.search(r"(?m)^void " + symbol + r"\(uint64_t handle\s*\);$", header):
        raise ValueError(f"unrecognized exported cancellation signature: {symbol}")
    pattern = re.compile(r"(?m)^( +)completeFunc: ffi_screenroom_client_core_rust_future_complete_rust_buffer,$")
    if len(list(pattern.finditer(generated))) != len(found):
        raise ValueError("incomplete generated async callsite coverage; inspect generator/API changes")
    source = source.replace(ORIGINAL, REPLACEMENT)
    updated, count = pattern.subn(lambda m: f"{m[0]}\n{m[1]}cancelFunc: {symbol},", source)
    if count != len(found):
        raise ValueError("incomplete generated async cancellation coverage; inspect generator/API changes")
    return updated


if __name__ == "__main__":
    directory = Path(sys.argv[1])
    swift = directory / "ScreenroomClientCore.swift"
    try:
        updated = patch(swift.read_text(), (directory / "ScreenroomClientCoreFFI.h").read_text())
    except ValueError as error:
        sys.exit(f"Swift cancellation correction refused: {error}")
    swift.write_text(updated)
