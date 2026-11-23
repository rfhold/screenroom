#!/usr/bin/env python3
"""Validate retained idle diagnostic evidence without application or backend access."""
from pathlib import Path
import json
import os
import runpy
import shutil
import struct
import subprocess
import sys
import zlib

DECISION_NUMBERS = {"sequence", "elapsed_ms", "idle_age_ms"}
DECISION_FLAGS = {
    "paused", "busy", "blocked", "preroll", "started", "menu_visible",
    "menu_mapped", "control_focus", "keyboard_engaged", "scrubbing", "show", "preparing",
}
GETTER_FLAGS = {
    "header_reveal_child", "header_child_revealed",
    "footer_reveal_child", "footer_child_revealed", "requested_show",
}
GETTER_KEYS = GETTER_FLAGS | {"sequence", "elapsed_ms"}


def require(condition, message):
    if not condition:
        raise ValueError(message)


def integer(value, maximum=2**128 - 1):
    return type(value) is int and 0 <= value <= maximum


def pairs(items):
    result = {}
    for key, value in items:
        require(key not in result, "Duplicate JSON field")
        result[key] = value
    return result


def decode(text):
    def invalid_constant(_):
        raise ValueError("Nonfinite JSON number")
    return json.loads(text, object_pairs_hook=pairs, parse_constant=invalid_constant)


def read(root, name, limit):
    path = root / name
    require(path.is_file() and not path.is_symlink(), "Missing or indirect evidence file")
    require(path.stat().st_size <= limit, "Evidence file exceeds its protocol bound")
    return path.read_bytes()


def object_keys(value, expected):
    require(type(value) is dict and value.keys() == expected, "Unexpected evidence schema")


def png_valid(data):
    require(data[:8] == b"\x89PNG\r\n\x1a\n", "Invalid PNG signature")
    position, kinds, compressed, header = 8, [], bytearray(), None
    while position < len(data):
        require(position + 12 <= len(data), "Truncated PNG chunk")
        length = struct.unpack(">I", data[position:position + 4])[0]
        end = position + 12 + length
        require(end <= len(data), "Truncated PNG payload")
        kind = data[position + 4:position + 8]
        require(all(65 <= byte <= 90 or 97 <= byte <= 122 for byte in kind)
                and (kind in [b"IHDR", b"PLTE", b"IDAT", b"IEND"] or kind[0] & 32),
                "Unknown critical PNG chunk")
        payload = data[position + 8:end - 4]
        crc = struct.unpack(">I", data[end - 4:end])[0]
        require(zlib.crc32(kind + payload) == crc, "Invalid PNG checksum")
        if not kinds:
            require(kind == b"IHDR" and length == 13, "Missing PNG header")
            header = struct.unpack(">IIBBBBB", payload)
            require(header[:3] == (1280, 800, 8) and header[3] in (2, 6)
                    and header[4:] == (0, 0, 0), "Unexpected PNG encoding or viewport")
        elif kind == b"IHDR":
            raise ValueError("Duplicate PNG header")
        if kind == b"IDAT":
            compressed.extend(payload)
        if kind == b"IEND":
            require(length == 0 and end == len(data), "Invalid PNG ending")
        kinds.append(kind)
        position = end
    require(kinds[-1:] == [b"IEND"] and compressed, "Incomplete PNG")
    width, height, _, color, *_ = header
    stride = width * (3 if color == 2 else 4) + 1
    expected = stride * height
    inflater = zlib.decompressobj()
    decoded = inflater.decompress(compressed, expected + 1)
    require(len(decoded) == expected and inflater.eof and not inflater.unused_data
            and not inflater.unconsumed_tail, "Invalid PNG image stream")
    require(all(decoded[index * stride] <= 4 for index in range(height)), "Invalid PNG row filter")


def rows(root, name, keys, flags):
    lines = read(root, name, 65536).decode().splitlines()
    require(1 <= len(lines) <= 128, "Invalid diagnostic record count")
    result = [decode(line) for line in lines]
    for index, row in enumerate(result, 1):
        object_keys(row, keys)
        require(integer(row["sequence"], 128) and row["sequence"] == index, "Invalid sample sequence")
        require(all(type(row[key]) is bool for key in flags), "Invalid sample flag type")
        require(integer(row["elapsed_ms"]), "Invalid sample clock")
        if index > 1:
            require(row["elapsed_ms"] >= result[index - 2]["elapsed_ms"] + 200,
                    "Invalid sample cadence")
    return result


def validate(root):
    root = Path(root)
    require((root / "playback").is_dir() and not (root / "playback").is_symlink()
            and not any((root / "playback").iterdir()), "Idle diagnostics must not export evidence")
    summary = decode(read(root, "playback-idle.json", 8192))
    expected_summary = {
        "schemaVersion": 1, "check": "idle", "status": "observed-hidden",
        "visual_review": "required", "hidden_observed": True,
        "hidden_after_image": True, "image_retained": True,
        "positive_hidden_proof": "same-active-playback-window-description",
        "candidate_evidence": "playback-idle-candidate.json",
    }
    object_keys(summary, expected_summary.keys())
    require(all(type(summary[key]) is type(value) and summary[key] == value
                for key, value in expected_summary.items()), "Idle proof summary is not positive")
    candidate = decode(read(root, "playback-idle-candidate.json", 8192))
    object_keys(candidate, {
        "schemaVersion", "check", "status", "role_order", "pre", "capture", "post",
        "candidate_retained", "getter_before", "getter_after", "finished_ms",
    })
    require(type(candidate["schemaVersion"]) is int and candidate["schemaVersion"] == 1
            and candidate["check"] == "public-idle-candidate" and candidate["status"] == "passed"
            and candidate["candidate_retained"] is True
            and candidate["role_order"] == ["button", "toggle button", "slider", "label"],
            "Candidate proof is incomplete")
    bounds = []
    for phase in ["pre", "capture", "post"]:
        value = candidate[phase]
        object_keys(value, {"state", "start_ms", "end_ms"} | ({"proof"} if phase != "capture" else set()))
        require(value["state"] == ("acquired" if phase == "capture" else "observed"),
                "Candidate phase is incomplete")
        require(integer(value["start_ms"], 10000) and integer(value["end_ms"], 10000)
                and value["start_ms"] <= value["end_ms"], "Candidate clock is invalid")
        bounds.extend([value["start_ms"], value["end_ms"]])
        if phase != "capture":
            proof = value["proof"]
            object_keys(proof, {
                "classification", "description", "eligible", "modal_count", "reason",
                "role_counts", "same_window", "selected_title_matches", "showing_role_counts",
            })
            require(proof["classification"] == "positive" and proof["description"] == "hidden"
                    and proof["reason"] == "hidden-description-and-no-chrome"
                    and proof["eligible"] is True and proof["same_window"] is True
                    and proof["selected_title_matches"] is True
                    and type(proof["modal_count"]) is int and proof["modal_count"] == 1,
                    "Candidate does not prove the same eligible selected hidden modal")
            for key in ["role_counts", "showing_role_counts"]:
                require(type(proof[key]) is list and len(proof[key]) == 4
                        and all(type(count) is int and count == 0 for count in proof[key]),
                        "Candidate contains visible or unknown chrome")
    bounds.append(candidate["finished_ms"])
    require(integer(candidate["finished_ms"], 10000) and bounds == sorted(bounds),
            "Candidate ordering or original 10000ms bound violated")
    image = read(root, "playback-idle-candidate.png", 20 * 1024 * 1024)
    require(image == read(root, "playback-idle.png", 20 * 1024 * 1024),
            "Retained idle image is not the exact candidate")
    png_valid(image)
    decisions = rows(root, "playback-idle-decisions.jsonl",
                     DECISION_NUMBERS | DECISION_FLAGS, DECISION_FLAGS)
    require(all(integer(row["idle_age_ms"]) for row in decisions), "Invalid idle age")
    getters = rows(root, "playback-idle-chrome.jsonl", GETTER_KEYS, GETTER_FLAGS)
    for receipt in getters:
        require(integer(receipt["elapsed_ms"], 25600), "Getter clock exceeds producer bound")
        index = receipt["sequence"] - 1
        require(index < len(decisions) and decisions[index]["elapsed_ms"] == receipt["elapsed_ms"]
                and decisions[index]["show"] == receipt["requested_show"],
                "Getter does not match its actual decision sample")
    # fresh_chrome_receipt checks <=500ms file age in the producer at each read.
    # Do not compare archived mtimes to now or align the two unrelated Instant clocks.
    for key in ["getter_before", "getter_after"]:
        receipt = candidate[key]
        object_keys(receipt, GETTER_KEYS)
        require(integer(receipt["sequence"], 128) and integer(receipt["elapsed_ms"], 25600)
                and 1 <= receipt["sequence"] <= len(getters)
                and receipt == getters[receipt["sequence"] - 1]
                and all(receipt[flag] is False for flag in GETTER_FLAGS),
                "Missing, orphaned or non-hidden actual getter receipt")
        decision = decisions[receipt["sequence"] - 1]
        require(decision["started"] and decision["idle_age_ms"] >= 3000
                and not any(decision[flag] for flag in
                            ["paused", "busy", "blocked", "preroll", "preparing", "menu_visible", "scrubbing"])
                and not (decision["control_focus"] and decision["keyboard_engaged"]),
                "Hidden receipt contradicts its actual visibility decision")
    require(candidate["getter_after"]["sequence"] >= candidate["getter_before"]["sequence"]
            and candidate["getter_after"]["elapsed_ms"] >= candidate["getter_before"]["elapsed_ms"],
            "Getter receipt order reversed")
    return candidate


def fixture_tests(actual, output):
    actual, output = Path(actual).resolve(), Path(output).resolve()
    output.mkdir(parents=True, exist_ok=True)
    validate(actual)
    names = [
        "playback-idle.json", "playback-idle-candidate.json", "playback-idle-candidate.png",
        "playback-idle.png", "playback-idle-chrome.jsonl", "playback-idle-decisions.jsonl",
    ]
    def payload(name):
        target = output / name
        target.mkdir()
        (target / "playback").mkdir()
        for item in names:
            shutil.copy2(actual / item, target / item)
        return target
    mutations = {}
    def change_json(name, file, edit):
        def change(root):
            path = root / file
            data = json.loads(path.read_text())
            edit(data)
            path.write_text(json.dumps(data))
        mutations[name] = change
    for phase, field, value in [
        ("pre", "classification", "unknown"), ("pre", "description", "visible"),
        ("post", "same_window", False), ("post", "selected_title_matches", False),
        ("pre", "eligible", None), ("post", "modal_count", 2),
        ("pre", "role_counts", [0, 1, 0, 0]),
    ]:
        change_json(f"{phase}-{field}", names[1], lambda d, p=phase, f=field, v=value: d[p]["proof"].__setitem__(f, v))
    change_json("proof-missing", names[1], lambda d: d["post"].pop("proof"))
    change_json("post-pending", names[1], lambda d: d["post"].__setitem__("state", "pending"))
    change_json("over-bound", names[1], lambda d: d.__setitem__("finished_ms", 10001))
    change_json("nonfinite", names[1], lambda d: d["pre"].__setitem__("start_ms", float("nan")))
    change_json("float-clock", names[1], lambda d: d["pre"].__setitem__("start_ms", 0.0))
    change_json("out-of-order", names[1], lambda d: d["capture"].__setitem__("start_ms", 0))
    change_json("not-retained", names[1], lambda d: d.__setitem__("candidate_retained", False))
    change_json("getter-visible", names[1], lambda d: d["getter_before"].__setitem__("header_child_revealed", True))
    change_json("getter-orphan", names[1], lambda d: d["getter_before"].__setitem__("elapsed_ms", 0))
    change_json("getter-float", names[1], lambda d: d["getter_before"].__setitem__("elapsed_ms", float(d["getter_before"]["elapsed_ms"])))
    change_json("getter-reversed", names[1], lambda d: d.update(getter_before=d["getter_after"], getter_after=d["getter_before"]))
    change_json("getter-missing", names[1], lambda d: d.__setitem__("getter_before", None))
    change_json("extra-raw-field", names[1], lambda d: d.__setitem__("url", "not-allowed"))
    change_json("orphan-summary", names[0], lambda d: d.__setitem__("candidate_evidence", "other.json"))
    change_json("visible-summary", names[0], lambda d: d.__setitem__("hidden_after_image", False))
    for file in names[1:]:
        mutations["missing-" + file] = lambda root, f=file: (root / f).unlink()
    mutations["truncated-png"] = lambda root: [(root / f).write_bytes((root / f).read_bytes()[:40]) for f in names[2:4]]
    def corrupt_png(root):
        data = bytearray((root / names[2]).read_bytes())
        data[-5] ^= 1
        for name in names[2:4]:
            (root / name).write_bytes(data)
    mutations["png-checksum"] = corrupt_png
    mutations["mismatched-pair"] = lambda root: (root / names[3]).write_bytes(b"not the candidate")
    mutations["export-orphan"] = lambda root: (root / "playback/stray.png").write_bytes(b"not-exportable")
    def bad_decisions(root, mode):
        path = root / names[5]
        data = [json.loads(line) for line in path.read_text().splitlines()]
        if mode == "over-cap":
            data = data * 3
        elif mode == "cadence":
            data[1]["elapsed_ms"] = data[0]["elapsed_ms"]
        elif mode == "boolean-sequence":
            data[0]["sequence"] = True
        elif mode == "raw-extra":
            data[0]["url"] = "not-allowed"
        elif mode == "contradiction":
            candidate = json.loads((root / names[1]).read_text())
            data[candidate["getter_before"]["sequence"] - 1]["started"] = False
        path.write_text("".join(json.dumps(row) + "\n" for row in data))
    for mode in ["over-cap", "cadence", "boolean-sequence", "raw-extra", "contradiction"]:
        mutations[mode] = lambda root, m=mode: bad_decisions(root, m)
    for name, mutate in mutations.items():
        target = payload(name)
        mutate(target)
        result = subprocess.run([sys.executable, __file__, "--validate", str(target)], capture_output=True, text=True)
        require(result.returncode != 0, "Negative evidence fixture was accepted: " + name)
    print(f"Actual retained successful idle evidence accepted; {len(mutations)} isolated negatives rejected.")
    return names


def runner_tests(actual, output, names):
    # Preserve the existing nine idle runner cases using the current producer schema.
    sys.argv = [__file__, str(output / "runner")]
    prior = runpy.run_path(str(Path(__file__).with_name("thread-limit-runner-test.py")))
    root, base, docker = (prior[key] for key in ["root", "base", "docker"])
    docker.write_text("""#!/usr/bin/env python3
import json, os, shutil, sys
with open(os.environ['DOCKER_SENTINEL'], 'a') as file: file.write(json.dumps(sys.argv[1:])+'\\n')
if sys.argv[1] == 'create': print('fixture-container')
elif sys.argv[1] == 'start':
    print('playback-stage: idle-observation end')
    print('playback-stage: normal-cleanup end')
    print('playback-stage: session-shutdown end')
    if os.environ['FIXTURE_CASE'] == 'motion': print('playback-stage: motion-comparison begin')
    print('test result: ok. 1 passed; 0 failed; 0 ignored;')
    sys.exit(23 if os.environ['FIXTURE_CASE'] == 'primary' else 0)
elif sys.argv[1] == 'cp': shutil.copytree(os.environ['FIXTURE_PAYLOAD'], sys.argv[3], dirs_exist_ok=True)
elif sys.argv[1] not in ['build','rm']: raise SystemExit('unexpected operation')
""")
    for case in ["valid", "visible", "missing-decisions", "over-cap", "raw-extra", "no-image", "manifest", "motion", "primary"]:
        payload, destination = output / f"runner-{case}-payload", output / f"runner-{case}"
        payload.mkdir()
        destination.mkdir()
        for name in names:
            shutil.copy2(actual / name, payload / name)
        if case == "visible":
            data = json.loads((payload / names[0]).read_text())
            data["hidden_after_image"] = False
            (payload / names[0]).write_text(json.dumps(data))
        if case == "missing-decisions":
            (payload / names[5]).unlink()
        if case in ["over-cap", "raw-extra"]:
            path = payload / names[5]
            data = [json.loads(line) for line in path.read_text().splitlines()]
            if case == "over-cap": data *= 3
            else: data[0]["url"] = "not-allowed"
            path.write_text("".join(json.dumps(row) + "\n" for row in data))
        if case == "no-image": (payload / names[3]).unlink()
        if case == "manifest":
            (payload / "playback").mkdir()
            (payload / "playback/manifest.json").write_text("{}")
        sentinel, backend = output / f"{case}-docker.jsonl", output / f"{case}-backend.log"
        sentinel.write_text("")
        backend.write_text("")
        env = {**base, "SCREENROOM_E2E_JOURNEY": "playback", "SCREENROOM_E2E_PLAYBACK_PHASE": "idle",
               "SCREENROOM_PLAYBACK_DIAGNOSTICS": "1", "DOCKER_SENTINEL": str(sentinel),
               "BACKEND_SENTINEL": str(backend), "FIXTURE_CASE": case, "FIXTURE_PAYLOAD": str(payload)}
        result = subprocess.run(["/bin/bash", str(root / "apps/linux/tests/e2e/run.sh"), str(destination)],
                                env=env, capture_output=True, text=True)
        (output / f"runner-{case}.log").write_text(result.stdout + result.stderr)
        require(result.returncode == (0 if case == "valid" else 23 if case == "primary" else 1),
                "Idle runner fixture returned unexpected status: " + case)
    print("Existing nine idle runner contracts passed with current-schema payload; no real Docker/backend/app.")


if __name__ == "__main__":
    try:
        if len(sys.argv) == 3 and sys.argv[1] == "--validate":
            validate(Path(sys.argv[2]))
            print("Positive idle candidate, exact PNG, native getter receipts and bounded observations verified.")
        elif len(sys.argv) == 4 and sys.argv[1] == "--test":
            actual, output = Path(sys.argv[2]).resolve(), Path(sys.argv[3]).resolve()
            names = fixture_tests(actual, output)
            runner_tests(actual, output, names)
        else:
            raise ValueError("Usage: --validate ARTIFACTS | --test ACTUAL_ARTIFACTS OUTPUT")
    except (ValueError, OSError, KeyError, TypeError, zlib.error, struct.error) as error:
        print(f"Idle diagnostic evidence invalid: {error}", file=sys.stderr)
        sys.exit(1)
