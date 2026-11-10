#!/usr/bin/env python3
"""Check host dispatch without building or modifying integration artifacts."""
import json
import os
from pathlib import Path
import shutil
import subprocess
import sys
import tempfile
import unittest

HELPER = Path(__file__).with_name("build-client-core.sh")


class HostTests(unittest.TestCase):
    def run_helper(self, host, command, arch="x86_64", override=None, wasm_tools=None, expected="0.2.130"):
        with tempfile.TemporaryDirectory(prefix="screenroom-helper-") as directory:
            root = Path(directory)
            (root / "scripts").mkdir()
            shutil.copyfile(HELPER, root / "scripts/build-client-core.sh")
            (root / "scripts/patch-swift-cancellation.py").write_text("# Host dispatch tests stub postprocessing; tested separately.\n")
            bin_dir = root / "bin"
            bin_dir.mkdir()
            log = root / "calls.jsonl"
            (root / "Cargo.lock").write_text(f'[[package]]\nname = "unrelated"\nversion = "9.9.9"\n[[package]]\nname = "wasm-bindgen"\nversion = "{expected}"\n[[package]]\nname = "wasm-bindgen-cli-support"\nversion = "0.0.1"\n')
            for name, source in {
                "uname": '#!/bin/sh\nif [ "$1" = -m ]; then echo "$TEST_ARCH"; else echo "$TEST_HOST"; fi\n',
                "cargo": f'#!{sys.executable}\nimport json,os,sys\nwith open(os.environ["TEST_LOG"],"a") as f: f.write(json.dumps({{"args":sys.argv[1:],"env":{{k:v for k,v in os.environ.items() if k.startswith("CARGO_TARGET_")}}}})+"\\n")\n',
                "cp": "#!/bin/sh\nexit 0\n",
            }.items():
                path = bin_dir / name
                path.write_text(source)
                path.chmod(0o755)
            home = root / "home"
            sdk = root / "override-sdk" if override else home / ("Library/Android/sdk" if host == "Darwin" else "Android/Sdk")
            ndk_host = "darwin-x86_64" if host == "Darwin" else "linux-x86_64"
            compiler = sdk / f"ndk/29.0.14206865/toolchains/llvm/prebuilt/{ndk_host}/bin/aarch64-linux-android26-clang"
            compiler.parent.mkdir(parents=True)
            compiler.write_text("#!/bin/sh\nexit 0\n")
            compiler.chmod(0o755)
            env = {k: v for k, v in os.environ.items() if k not in {"ANDROID_HOME", "ANDROID_SDK_ROOT", "ANDROID_NDK_HOME", "SCREENROOM_CORE_PROFILE", "WASM_BINDGEN"}}
            env.update(HOME=str(home), PATH=f"{bin_dir}:{os.environ['PATH']}", TEST_LOG=str(log), TEST_HOST=host, TEST_ARCH=arch)
            tools = {"path": "0.2.122", **(wasm_tools or {})}
            for role, version in tools.items():
                path = {"path": bin_dir / "wasm-bindgen", "cache": root / "target/client-core/tools/bin/wasm-bindgen", "explicit": root / "explicit-bindgen"}[role]
                if role == "explicit":
                    env["WASM_BINDGEN"] = str(path)
                if version == "missing":
                    continue
                path.parent.mkdir(parents=True, exist_ok=True)
                path.write_text(f'#!{sys.executable}\nimport json,os,sys\nif sys.argv[1:]==["--version"]: print("wasm-bindgen {version}")\nelse:\n with open(os.environ["TEST_LOG"],"a") as f: f.write(json.dumps({{"tool":"{role}","args":sys.argv[1:]}})+"\\n")\n')
                path.chmod(0o755)
            if override:
                env[override] = str(sdk)
            result = subprocess.run(["bash", str(root / "scripts/build-client-core.sh"), command], env=env, capture_output=True, text=True)
            calls = [json.loads(line) for line in log.read_text().splitlines()] if log.exists() else []
            return result, calls, str(sdk)

    def test_native_binding_suffixes(self):
        for host, suffix in [("Linux", "so"), ("Darwin", "dylib")]:
            result, calls, _ = self.run_helper(host, "bindings")
            self.assertEqual(result.returncode, 0, result.stderr)
            self.assertIn(f"target/debug/libscreenroom_client_core.{suffix}", calls[1]["args"])

    def test_android_sdk_and_ndk_hosts(self):
        for host, override in [("Linux", None), ("Darwin", None), ("Linux", "ANDROID_HOME"), ("Linux", "ANDROID_SDK_ROOT")]:
            result, calls, sdk = self.run_helper(host, "android", override=override)
            self.assertEqual(result.returncode, 0, result.stderr)
            self.assertEqual(len(calls), 4)
            ndk_host = "linux-x86_64" if host == "Linux" else "darwin-x86_64"
            for call in calls[2:]:
                self.assertTrue(any(value.startswith(f"{sdk}/ndk/29.0.14206865/toolchains/llvm/prebuilt/{ndk_host}/bin/") for value in call["env"].values()))

    def test_unsupported_hosts_fail_before_building(self):
        for host, command, arch, message in [("FreeBSD", "native", "x86_64", "unsupported build host"), ("Linux", "apple", "x86_64", "require a macOS host"), ("Linux", "android", "aarch64", "unsupported Android NDK host")]:
            result, calls, _ = self.run_helper(host, command, arch)
            self.assertEqual(result.returncode, 2)
            self.assertIn(message, result.stderr)
            self.assertEqual(calls, [])

    def test_wasm_explicit_override_precedes_cached_default(self):
        result, calls, _ = self.run_helper("Darwin", "wasm", wasm_tools={"cache": "0.2.122", "explicit": "0.2.130"})
        self.assertEqual(result.returncode, 0, result.stderr)
        self.assertEqual(calls[-1]["tool"], "explicit")
        for version in ["0.2.122", "missing"]:
            result, calls, _ = self.run_helper("Darwin", "wasm", wasm_tools={"cache": "0.2.130", "explicit": version})
            self.assertEqual(result.returncode, 2)
            self.assertEqual(calls, [])
            self.assertIn("Unset WASM_BINDGEN", result.stderr)

    def test_wasm_cache_path_fallback_and_lock_derived_version(self):
        for tools, selected, expected in [({"cache": "0.2.130"}, "cache", "0.2.130"), ({"path": "0.2.130"}, "path", "0.2.130"), ({"cache": "0.2.999"}, "cache", "0.2.999")]:
            result, calls, _ = self.run_helper("Linux", "wasm", wasm_tools=tools, expected=expected)
            self.assertEqual(result.returncode, 0, result.stderr)
            self.assertEqual(calls[-1]["tool"], selected)

    def test_wasm_mismatch_is_actionable_and_fails_before_build(self):
        result, calls, _ = self.run_helper("Linux", "wasm")
        self.assertEqual(result.returncode, 2)
        self.assertEqual(calls, [])
        self.assertIn("0.2.122", result.stderr)
        self.assertIn("Cargo.lock requires 0.2.130", result.stderr)
        self.assertIn("cargo install --locked --version 0.2.130 wasm-bindgen-cli --root", result.stderr)


if __name__ == "__main__":
    unittest.main()
