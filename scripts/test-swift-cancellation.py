#!/usr/bin/env python3
"""Compile Swift and exercise Task cancellation against a delayed native gRPC service."""
import argparse
import os
from pathlib import Path
import shutil
import subprocess
import sys

ROOT = Path(__file__).resolve().parent.parent
os.chdir(ROOT)
if sys.platform != "darwin":
    sys.exit("Swift cancellation smoke currently requires the macOS host Swift toolchain")
parser = argparse.ArgumentParser()
parser.add_argument("--isolated", action="store_true", help="use the existing dylib/generator; never touch shared bindings or libraries")
args = parser.parse_args()
profile = os.environ.get("SCREENROOM_CORE_PROFILE", "debug")
output = ROOT / ("target/client-core/swift-cancellation-isolated" if args.isolated else "target/client-core/swift-cancellation")
output.mkdir(parents=True, exist_ok=True)
if args.isolated:
    bindings = output / "bindings"
    subprocess.run([
        f"target/{profile}/screenroom-uniffi-bindgen", "generate", "--library", f"target/{profile}/libscreenroom_client_core.dylib",
        "--language", "swift", "--language", "kotlin", "--out-dir", str(bindings),
        "--config", "crates/screenroom-client-core/uniffi.toml", "--no-format",
    ], check=True)
    subprocess.run([sys.executable, "scripts/patch-swift-cancellation.py", str(bindings)], check=True)
else:
    subprocess.run(["bash", "scripts/build-client-core.sh", "bindings"], check=True)
    bindings = ROOT / "target/client-core/bindings"
shutil.copyfile(bindings / "ScreenroomClientCoreFFI.h", output / "ScreenroomClientCoreFFI.h")
shutil.copyfile(bindings / "ScreenroomClientCoreFFI.modulemap", output / "module.modulemap")
binary = output / "swift-cancellation-smoke"
source = output / "CancellationSmoke.swift"
combined = "\n".join(path.read_text() for path in [
    bindings / "ScreenroomClientCore.swift", ROOT / "scripts/swift-future-lifecycle-smoke.swift",
    ROOT / "scripts/swift-buffer-ownership-smoke.swift",
    ROOT / "scripts/swift-cancellation-smoke.swift",
])
needle = "try! rustCall { ffi_screenroom_client_core_rustbuffer_free(self, $0) }"
assert combined.count(needle) == 1, "test allocation observer requires the pinned RustBuffer deallocator"
source.write_text(combined.replace(needle, needle + "\n        screenroomObserveRustBufferFree(self)"))
def compile_swift(source, binary):
    subprocess.run([
        "swiftc", "-swift-version", "6", "-parse-as-library", "-I", str(output), "-L", f"target/{profile}",
        "-lscreenroom_client_core", "-Xlinker", "-rpath", "-Xlinker", str(ROOT / f"target/{profile}"),
        str(source), "-o", str(binary),
    ], check=True)

compile_swift(source, binary)
env = dict(os.environ, SCREENROOM_SWIFT_TEST_BINARY=str(binary))
subprocess.run([
    "cargo", "test", "--locked", "-p", "screenroom-client-core", "--test", "swift_cancellation",
    "--", "--ignored", "--nocapture",
], check=True, env=env, timeout=120)
negative_source = output / "OwnershipNegativeControl.swift"
negative_binary = output / "ownership-negative-control"
discard = "returned.deallocate()\n                status.errorBuf.deallocate()"
control = source.read_text()
assert control.count(discard) == 1, "negative control requires the pinned completion destructor path"
negative_source.write_text(control.replace(discard, "_ = returned // Negative control: deliberately omit both destructors.").replace("try await runLifecycleRegression()", ""))
compile_swift(negative_source, negative_binary)
negative = subprocess.run([
    "cargo", "test", "--locked", "-p", "screenroom-client-core", "--test", "swift_cancellation",
    "--", "--ignored", "--nocapture",
], env=dict(env, SCREENROOM_SWIFT_TEST_BINARY=str(negative_binary)), text=True, capture_output=True, timeout=120)
diagnostic = negative.stdout + negative.stderr
(output / "ownership-negative-control.log").write_text(diagnostic)
if negative.returncode == 0 or "lowered RustBuffer not destroyed exactly once" not in diagnostic:
    sys.exit("RustBuffer ownership negative control did not detect the deliberate leak:\n" + diagnostic)
print("RustBuffer ownership negative control PASS: deliberately omitted destructors are detected")
