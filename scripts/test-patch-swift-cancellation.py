#!/usr/bin/env python3
"""Check the Swift correction refuses drift before writing generated output."""
import importlib.util
import os
from pathlib import Path
import sys
import unittest

sys.dont_write_bytecode = True
spec = importlib.util.spec_from_file_location("correction", Path(__file__).with_name("patch-swift-cancellation.py"))
correction = importlib.util.module_from_spec(spec)
spec.loader.exec_module(correction)


class PatchTests(unittest.TestCase):
    @staticmethod
    def method(owner="client", name="get_item"):
        return f'''open func {name}(id: String)async throws  -> Item  {{
    return
        try  await uniffiRustCallAsync(
            rustFutureFunc: {{
                uniffi_screenroom_client_core_fn_method_{owner}_{name}(
                    self.uniffiCloneHandle(),
                    FfiConverterString.lower(id)
                )
            }},
            pollFunc: ffi_screenroom_client_core_rust_future_poll_rust_buffer,
            completeFunc: ffi_screenroom_client_core_rust_future_complete_rust_buffer,
            freeFunc: ffi_screenroom_client_core_rust_future_free_rust_buffer,
            liftFunc: FfiConverterTypeItem_lift,
            errorHandler: FfiConverterTypeClientError_lift
        )
}}
'''

    def setUp(self):
        self.call = self.method()
        self.source = (correction.ORIGINAL + "\nopen class Client {\n" + self.call
                       + "}\nopen class OperatorClient {\n" + self.method("operatorclient", "indexing_jobs") + "}\n")
        self.header = "void ffi_screenroom_client_core_rust_future_cancel_rust_buffer(uint64_t handle);"

    def test_deterministic_and_all_calls_cancel_individually(self):
        result = correction.patch(self.source, self.header)
        self.assertEqual(result, correction.patch(self.source, self.header))
        self.assertEqual(result.count("cancelFunc: ffi_screenroom_client_core_rust_future_cancel_rust_buffer,"), 2)
        self.assertNotIn("cancel_all", result)

    def test_new_and_removed_methods_require_no_fixed_count(self):
        for count in [1, 2, 16, 17, 18, 50]:
            with self.subTest(count=count):
                source = correction.ORIGINAL + "\nopen class Client {\n" + "".join(
                    self.method(name=f"new_method_{i}") for i in range(count)
                ) + "}\n"
                result = correction.patch(source, self.header)
                self.assertEqual(result.count("cancelFunc: ffi_screenroom_client_core_rust_future_cancel_rust_buffer,"), count)
        source = self.source.replace(self.call, "")
        self.assertEqual(correction.patch(source, self.header).count("cancelFunc: ffi_screenroom_client_core_rust_future_cancel_rust_buffer,"), 1)

    def test_zero_async_methods_refused(self):
        for source in [correction.ORIGINAL, correction.ORIGINAL + "\nopen class Client {}\n"]:
            with self.assertRaisesRegex(ValueError, "missing or unmatched"):
                correction.patch(source, self.header)

    def test_bridge_drift_or_double_patch_refused(self):
        for source in [self.source.replace("var pollResult: Int8;", "var pollResult: Int8"), self.source + correction.ORIGINAL, correction.patch(self.source, self.header)]:
            with self.assertRaisesRegex(ValueError, "exactly one unmodified"):
                correction.patch(source, self.header)

    def test_continuation_resumption_leaves_the_rust_scheduler(self):
        result = correction.patch(self.source, self.header)
        self.assertIn("""{ handle, pollResult in
                        // Rust holds its scheduler lock; resuming here can invert Swift's cancellation lock.
                        DispatchQueue.global().async {
                            uniffiFutureContinuationCallback(handle: handle, pollResult: pollResult)
                        }
                    }""", result)
        self.assertEqual(result.count("DispatchQueue.global().async"), 1)

    def test_call_set_drift_refused(self):
        for source in [
            self.source.replace("await uniffiRustCallAsync(", "await changedBridge(", 1),
            self.source.replace("completeFunc: ffi_", "changedCompleteFunc: ffi_", 1),
            self.source.replace("rustFutureFunc: {", "changedFutureFunc: {", 1),
            self.source.replace("pollFunc: ffi_", "changedPollFunc: ffi_", 1),
            self.source.replace("freeFunc: ffi_", "changedFreeFunc: ffi_", 1),
            self.source.replace("liftFunc: FfiConverterTypeItem_lift", "liftFunc: arbitraryLift", 1),
            self.source.replace("try  await", "try  ", 1),
            self.source.replace("    return\n", "", 1),
            self.source.replace("open func get_item", "fileprivate func get_item", 1),
            self.source.replace(self.call, "open func missing()async throws -> Item {\n}\n"),
            self.source.replace("        )\n}", "        )\n        await uniffiRustCallAsync()\n}", 1),
            self.source + "\nawait uniffiRustCallAsync()\n",
            self.source + "\ncompleteFunc: ffi_screenroom_client_core_rust_future_complete_rust_buffer,\n",
            self.source + "\nffi_screenroom_client_core_rust_future_unknown_rust_buffer(handle)\n",
        ]:
            with self.subTest(source=source):
                with self.assertRaisesRegex(ValueError, "unrecognized|unmatched"):
                    correction.patch(source, self.header)

    def test_missing_or_ambiguous_ffi_symbol_refused(self):
        for header in ["", self.header * 2]:
            with self.assertRaisesRegex(ValueError, "exported cancellation symbol"):
                correction.patch(self.source, header)

    def test_unrecognized_header_signature_refused(self):
        for header in [self.header.replace("void ", "uint64_t "), self.header.replace("uint64_t handle", "uint32_t handle"),
                       self.header.replace("uint64_t handle", "uint64_t handle, uint64_t extra"), self.header.rstrip(";")]:
            with self.subTest(header=header):
                with self.assertRaisesRegex(ValueError, "cancellation signature"):
                    correction.patch(self.source, header)

    def test_non_buffer_return_refused(self):
        for operation in ["poll", "complete", "free"]:
            source = self.source.replace(f"future_{operation}_rust_buffer", f"future_{operation}_uint64", 1)
            with self.subTest(operation=operation):
                with self.assertRaisesRegex(ValueError, "only supports RustBuffer"):
                    correction.patch(source, self.header)

    def test_unknown_nested_future_symbols_refused(self):
        source = self.source.replace("FfiConverterString.lower(id)", "ffi_screenroom_client_core_rust_future_poll_unknown(id)", 1)
        with self.assertRaisesRegex(ValueError, "unmatched"):
            correction.patch(source, self.header)

    def test_cancelled_path_completes_and_destroys_both_buffers(self):
        result = correction.patch(self.source, self.header)
        self.assertIn("discardCompletion(handle)", result)
        self.assertIn("let returned = completeFunc(handle, &status)", result)
        self.assertIn("returned.deallocate()\n                status.errorBuf.deallocate()", result)
        self.assertIn("if !completed {\n                completed = true", result)

    @unittest.skipUnless(os.environ.get("SCREENROOM_SWIFT_RAW_BINDINGS"), "set SCREENROOM_SWIFT_RAW_BINDINGS to freshly generated, unmodified bindings")
    def test_current_generated_bindings(self):
        directory = Path(os.environ["SCREENROOM_SWIFT_RAW_BINDINGS"])
        source = (directory / "ScreenroomClientCore.swift").read_text()
        header = (directory / "ScreenroomClientCoreFFI.h").read_text()
        self.assertRegex(source, r"open class Client\b")
        self.assertRegex(source, r"open class OperatorClient\b")
        count = source.count("await uniffiRustCallAsync(")
        self.assertGreater(count, 0)
        result = correction.patch(source, header)
        self.assertEqual(result.count("cancelFunc: ffi_screenroom_client_core_rust_future_cancel_rust_buffer,"), count)
        self.assertNotIn(correction.ORIGINAL, result)
        self.assertIn(correction.REPLACEMENT, result)


if __name__ == "__main__":
    unittest.main()
