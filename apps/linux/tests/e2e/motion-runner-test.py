#!/usr/bin/env python3
"""Test motion guards with synthetic PNGs and fake Docker/backend, never an app."""
from pathlib import Path
import copy
import json
import runpy
import struct
import subprocess
import zlib

prior = runpy.run_path(str(Path(__file__).with_name("thread-limit-runner-test.py")))
output, root, base = prior["output"], prior["root"], prior["base"]
docker = prior["docker"]
docker.write_text("""#!/usr/bin/env python3
import json, os, shutil, sys
with open(os.environ['DOCKER_SENTINEL'], 'a') as target:
    target.write(json.dumps(sys.argv[1:]) + '\\n')
if sys.argv[1] == 'create':
    print('fixture-container')
elif sys.argv[1] == 'start':
    print('playback-stage: motion-comparison end')
    print('playback-stage: normal-cleanup end')
    print('playback-stage: session-shutdown end')
    if os.environ['MOTION_FIXTURE_CASE'] == 'controls':
        print('playback-stage: captures-controls begin')
    print('test result: ok. 1 passed; 0 failed; 0 ignored;')
    sys.exit(23 if os.environ['MOTION_FIXTURE_CASE'] == 'primary-failure' else 0)
elif sys.argv[1] == 'cp':
    shutil.copytree(os.environ['MOTION_FIXTURE_PAYLOAD'], sys.argv[3], dirs_exist_ok=True)
elif sys.argv[1] not in ['build', 'rm']:
    raise SystemExit('unexpected Docker operation')
""")
docker.chmod(0o755)


def png(changed):
    pixels = bytearray()
    for y in range(800):
        pixels.append(0)
        for x in range(1280):
            red = x % 251
            if changed and 260 <= y < 540 and 420 <= x < 860 and (y - 260) * 440 + x - 420 < 1000:
                red = (red + 13) % 256
            pixels.extend((red, y % 251, (x + y) % 251))

    def chunk(kind, data):
        return struct.pack(">I", len(data)) + kind + data + struct.pack(">I", zlib.crc32(kind + data))

    return (b"\x89PNG\r\n\x1a\n" +
            chunk(b"IHDR", struct.pack(">IIBBBBB", 1280, 800, 8, 2, 0, 0, 0)) +
            chunk(b"IDAT", zlib.compress(pixels)) + chunk(b"IEND", b""))


first, second = png(False), png(True)
data = {
    "schemaVersion": 1, "check": "motion", "status": "passed", "viewport": [1280, 800],
    "region": {"x": 420, "y": 260, "width": 440, "height": 280},
    "total_pixels": 123200, "channel_delta_strictly_greater_than": 12,
    "required_changed_pixels": 1000, "requested_sleep_ms": 2000, "changed_pixels": 1000,
    "samples": [
        {"file": "playback-motion-first.png", "capture_start_ms": 2, "capture_end_ms": 3,
         "capture_status": "acquired", "validation": "passed", "dimensions": [1280, 800], "retained": True},
        {"file": "playback-motion-second.png", "capture_start_ms": 2003, "capture_end_ms": 2004,
         "capture_status": "acquired", "validation": "passed", "dimensions": [1280, 800], "retained": True},
    ],
    "timeline": {
        "before": {"elapsed_start_ms": 0, "elapsed_end_ms": 1, "status": "observed", "position_ms": 1015962},
        "after": {"elapsed_start_ms": 2005, "elapsed_end_ms": 2006, "status": "observed", "position_ms": 1018965},
        "same_slider": True,
    },
}
for name in ["valid", "missing-first", "missing-second", "below-threshold", "short-sleep",
             "nonfinite", "wrong-order", "manifest", "controls", "primary-failure"]:
    payload, destination = output / f"motion-{name}-payload", output / f"motion-{name}"
    payload.mkdir(exist_ok=True)
    destination.mkdir(exist_ok=True)
    fixture = copy.deepcopy(data)
    if name in ["below-threshold", "primary-failure"]:
        fixture["changed_pixels"] = 999
        fixture["status"] = "comparison-failed"
    if name == "short-sleep":
        fixture["samples"][1]["capture_start_ms"] -= 1
    if name == "nonfinite":
        fixture["timeline"]["after"]["position_ms"] = float("nan")
    if name == "wrong-order":
        fixture["timeline"]["after"]["elapsed_start_ms"] = 1
    (payload / "playback-motion.json").write_text(json.dumps(fixture))
    for slot, content in [("first", first), ("second", second)]:
        if name != f"missing-{slot}":
            (payload / f"playback-motion-{slot}.png").write_bytes(content)
    if name == "manifest":
        (payload / "playback").mkdir(exist_ok=True)
        (payload / "playback/manifest.json").write_text("{}")
    sentinel, backend = output / f"motion-{name}-docker.jsonl", output / f"motion-{name}-backend.log"
    sentinel.write_text("")
    backend.write_text("")
    env = {**base, "SCREENROOM_E2E_JOURNEY": "playback", "SCREENROOM_E2E_PLAYBACK_PHASE": "motion",
           "SCREENROOM_PLAYBACK_DIAGNOSTICS": "1", "DOCKER_SENTINEL": str(sentinel),
           "BACKEND_SENTINEL": str(backend), "MOTION_FIXTURE_CASE": name,
           "MOTION_FIXTURE_PAYLOAD": str(payload)}
    result = subprocess.run(["/bin/bash", str(root / "apps/linux/tests/e2e/run.sh"), str(destination)],
                            env=env, capture_output=True, text=True)
    (output / f"motion-{name}.log").write_text(result.stdout + result.stderr)
    expected = 0 if name == "valid" else 23 if name == "primary-failure" else 1
    assert result.returncode == expected, (name, result.returncode, result.stderr)
    creates = [json.loads(line) for line in sentinel.read_text().splitlines()
               if json.loads(line)[0] == "create"]
    assert len(creates) == 1
    arguments = creates[0]
    for entry in ["SCREENROOM_E2E_PLAYBACK_PHASE=motion", "SCREENROOM_PLAYBACK_DIAGNOSTICS=1"]:
        assert entry in arguments
    assert arguments[arguments.index("--pids-limit") + 1] == "512"
    assert not (destination / "playback/manifest.json").exists()
    for slot, content in [("first", first), ("second", second)]:
        if name != f"missing-{slot}":
            assert (destination / f"playback-motion-{slot}.png").read_bytes() == content
    print(f"motion {name}: exit {expected}; diagnostic pair never promoted to export")
print("Synthetic fixtures only; zero real Docker/backend/application calls.")
