#!/usr/bin/env python3
"""Check the Swift correction refuses drift before writing generated output."""
import importlib.util
from pathlib import Path
import sys
import unittest

sys.dont_write_bytecode = True
spec = importlib.util.spec_from_file_location("correction", Path(__file__).with_name("patch-swift-cancellation.py"))
correction = importlib.util.module_from_spec(spec)
spec.loader.exec_module(correction)


class PatchTests(unittest.TestCase):
    def setUp(self):
        self.call = "await uniffiRustCallAsync(\n    completeFunc: ffi_screenroom_client_core_rust_future_complete_rust_buffer,\n)\n"
        self.source = correction.ORIGINAL + "\n" + self.call * 12
        self.header = "void ffi_screenroom_client_core_rust_future_cancel_rust_buffer(uint64_t handle);"

    def test_deterministic_and_all_calls_cancel_individually(self):
        result = correction.patch(self.source, self.header)
        self.assertEqual(result, correction.patch(self.source, self.header))
        self.assertEqual(result.count("cancelFunc: ffi_screenroom_client_core_rust_future_cancel_rust_buffer,"), 12)
        self.assertNotIn("cancel_all", result)

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
        for source in [self.source + self.call, self.source.replace(self.call, "", 1)]:
            with self.assertRaisesRegex(ValueError, "exactly 12"):
                correction.patch(source, self.header)

    def test_missing_or_ambiguous_ffi_symbol_refused(self):
        for header in ["", self.header * 2]:
            with self.assertRaisesRegex(ValueError, "exported cancellation symbol"):
                correction.patch(self.source, header)

    def test_non_buffer_return_refused(self):
        source = self.source.replace("future_complete_rust_buffer", "future_complete_uint64")
        with self.assertRaisesRegex(ValueError, "only supports RustBuffer"):
            correction.patch(source, self.header)

    def test_cancelled_path_completes_and_destroys_both_buffers(self):
        result = correction.patch(self.source, self.header)
        self.assertIn("discardCompletion(handle)", result)
        self.assertIn("let returned = completeFunc(handle, &status)", result)
        self.assertIn("returned.deallocate()\n                status.errorBuf.deallocate()", result)
        self.assertIn("if !completed {\n                completed = true", result)


if __name__ == "__main__":
    unittest.main()
