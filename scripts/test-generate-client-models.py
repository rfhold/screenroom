#!/usr/bin/env python3
"""Isolated playback generation contract checks; no runtime or provider access."""
import hashlib
import json
import re
import subprocess
import unittest
from pathlib import Path

ROOT = Path(__file__).resolve().parents[1]
PROTO = (ROOT / "proto/screenroom/playback_gateway/v1/playback_gateway.proto").read_text()
RUST = (ROOT / "crates/screenroom-client-core/src/models.rs").read_text()
TS = (ROOT / "crates/screenroom-client-core/src/wasm-models.ts").read_text()
MESSAGES = dict(re.findall(r"message (\w+)\s*\{([^}]*)\}", PROTO))
ENUMS = dict(re.findall(r"enum (\w+)\s*\{([^}]*)\}", PROTO))


def rust_fields(name):
    return dict(re.findall(r"pub (\w+): (.+),", re.search(
        rf"pub struct {name} \{{([^}}]*)\}}", RUST
    ).group(1)))


def ts_fields(name):
    return dict(re.findall(r"  (\w+): (.+);", re.search(
        rf"export interface {name} \{{([^}}]*)\}}", TS
    ).group(1)))


class PlaybackModels(unittest.TestCase):
    def test_generator_has_no_drift(self):
        subprocess.run(
            ["python3", "scripts/generate-client-models.py", "--check"],
            cwd=ROOT, check=True,
        )

    def test_every_playback_record_includes_requests_and_responses(self):
        for name in MESSAGES:
            self.assertIn(f"pub struct {name} {{", RUST)
            self.assertIn(f"export interface {name} {{", TS)
        self.assertEqual(len(re.findall(r"\brpc ", PROTO)), 5)

    def test_all_timeline_fields_remain_strings(self):
        count = 0
        for name, body in MESSAGES.items():
            for optional, kind, field in re.findall(
                r"\b(?:(optional)\s+)?(\w+)\s+(\w+_ms)\s*=\s*\d+;", body
            ):
                count += 1
                self.assertEqual(kind, "string", f"{name}.{field}")
                self.assertEqual(rust_fields(name)[field], "Option<String>" if optional else "String")
                self.assertEqual(ts_fields(name)[field], "string | null" if optional else "string")
        self.assertEqual(count, 7)

    def test_wire_round_trip_preserves_wide_strings_and_optional_presence(self):
        command = [
            "buf", "convert", "--type",
            "screenroom.playback_gateway.v1.PlaybackTransport",
        ]
        # Wire precision precedes runtime decimal/tick-range validation.
        for duration in (None, "", "0", "9007199254740993", "18446744073709551615"):
            payload = {"startPositionMs": "9007199254740993", "timelineOffsetMs": "0"}
            if duration is not None:
                payload["durationMs"] = duration
            binary = subprocess.run(
                command + ["--from=-#format=json", "--to=-#format=binpb"],
                input=json.dumps(payload).encode(), cwd=ROOT, check=True,
                capture_output=True,
            ).stdout
            output = subprocess.run(
                command + ["--from=-#format=binpb", "--to=-#format=json"],
                input=binary, cwd=ROOT, check=True, capture_output=True,
            ).stdout
            decoded = json.loads(output)
            self.assertEqual(decoded["startPositionMs"], "9007199254740993")
            self.assertEqual(decoded["timelineOffsetMs"], "0")
            self.assertEqual("durationMs" in decoded, duration is not None)
            if duration is not None:
                self.assertEqual(decoded["durationMs"], duration)

    def test_nullability_and_bounded_integers(self):
        for name, body in MESSAGES.items():
            for mode, kind, field in re.findall(
                r"\b(?:(optional|repeated)\s+)?([\w.]+)\s+(\w+)\s*=\s*\d+;", body
            ):
                if kind == "uint32":
                    self.assertEqual(rust_fields(name)[field], "Option<u32>" if mode == "optional" else "u32")
                    self.assertEqual(ts_fields(name)[field], "number | null" if mode == "optional" else "number")
                if mode == "optional" or (kind in MESSAGES and mode != "repeated"):
                    self.assertTrue(rust_fields(name)[field].startswith("Option<"))
                    self.assertTrue(ts_fields(name)[field].endswith(" | null"))
        self.assertEqual(rust_fields("PlaybackSession")["tracks"], "Vec<PlaybackTrack>")
        self.assertEqual(ts_fields("PlaybackSession")["tracks"], "Array<PlaybackTrack>")

    def test_closed_enums_have_native_and_typescript_parity(self):
        for name, body in ENUMS.items():
            prefix = re.sub(r"(?<!^)(?=[A-Z])", "_", name).upper() + "_"
            values = [value[len(prefix):].lower() for value in re.findall(r"\b(\w+)\s*=\s*\d+;", body)]
            rust_enum = re.search(rf"pub enum {name} \{{([^}}]*)\}}", RUST).group(1)
            self.assertEqual(re.findall(r'#\[serde\(rename = "([^"]+)"\)\]', rust_enum), values)
            self.assertIn(f"export type {name} = " + " | ".join(f'"{v}"' for v in values) + ";", TS)
            self.assertNotIn(f"Option<{name}>", rust_fields("CreatePlaybackSessionRequest").get("start_policy", ""))
        self.assertIn('derive(uniffi::Enum)', RUST)
        self.assertEqual(rust_fields("PlaybackTrack")["subtitle_mode"], "Option<PlaybackSubtitleMode>")

    def test_control_authority_is_separate_from_media(self):
        for name in MESSAGES:
            if name.endswith("Request") and name != "CreatePlaybackSessionRequest":
                self.assertEqual(rust_fields(name)["control_capability"], "String")
                self.assertEqual(rust_fields(name)["session_id"], "String")
        self.assertNotIn("control_capability", rust_fields("CreatePlaybackSessionRequest"))
        self.assertNotIn("control_capability", rust_fields("PlaybackTransport"))
        for forbidden in ("access_token", "api_key", "upstream_url", "play_session_id", "device_id"):
            self.assertNotRegex(PROTO, rf"\b{forbidden}\s*=")

    def test_legacy_models_are_byte_identical(self):
        # Baselines cover all pre-playback records, including settings/config.
        playback_names = set(MESSAGES) | set(ENUMS)
        rust = re.sub(
            r"#\[derive[^\n]+\]\n#\[cfg_attr[^\n]+\]\npub (?:struct|enum) (\w+) \{[^}]*\}\n\n",
            lambda match: "" if match[1] in playback_names else match[0],
            RUST,
        )
        ts = re.sub(
            r"export (?:interface (\w+) \{[^}]*\}\n|type (\w+) = [^\n]+\n)",
            lambda match: "" if (match[1] or match[2]) in playback_names else match[0],
            TS,
        )
        self.assertEqual(hashlib.sha256(rust.encode()).hexdigest(), "899460e76fd97167667e6b53cceb94d78872827d82b3a64d2a60c1bea61b054d")
        self.assertEqual(hashlib.sha256(ts.encode()).hexdigest(), "6ec0001936f2af5c80dfd17a94aa4f7a8015af091615f0f440b5ea67035d5efe")


if __name__ == "__main__":
    unittest.main()
