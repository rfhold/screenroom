#!/usr/bin/env python3
"""Validate bounded control provenance; fixture mode never launches an application."""
from pathlib import Path
import copy
import json
import os
import runpy
import subprocess
import sys

STEPS = [
    "motion", "playing-reveal", "playing-image", "idle-image", "pause-reveal",
    "pause-key", "play-visible", "play-enabled", "pause-stability", "paused-image",
    "settings-open", "volume-visible", "settings-image", "volume-home", "volume-end",
    "audio-visible", "subtitles-visible", "tracks-image", "track-choices",
    "settings-close",
]
OPERATIONS = {"step", "focus", "tab", "terminal", "menu-confirmation",
              "value-bound", "track-settlement"}
FIELDS = {"sequence", "elapsed_ms", "step", "operation", "event", "key",
          "eligible", "focused", "terminal_attempts"}


def unique(pairs):
    result = {}
    for key, value in pairs:
        assert key not in result, "Duplicate diagnostic field"
        result[key] = value
    return result


def validate(root):
    assert not any((root / "playback").iterdir()), "Control probes cannot export"
    with (root / "playback-controls.jsonl").open("rb") as source:
        raw = source.read(65537)
    assert len(raw) <= 65536 and raw.endswith(b"\n")
    rows = [json.loads(line, object_pairs_hook=unique) for line in raw.splitlines()]
    assert 1 <= len(rows) <= 256
    stack, completed, attempts, elapsed = [], [], 0, 0
    focused = False
    previous = None
    for sequence, row in enumerate(rows, 1):
        assert set(row) == FIELDS
        assert type(row["sequence"]) is int and row["sequence"] == sequence
        assert type(row["elapsed_ms"]) is int and elapsed <= row["elapsed_ms"] <= 140000
        elapsed = row["elapsed_ms"]
        step, operation, event, key = (row[k] for k in ["step", "operation", "event", "key"])
        assert step in STEPS + ["prefix", "cleanup"] and operation in OPERATIONS
        assert event in ["begin", "end", "observed"], "Failed provenance cannot pass"
        assert key in ["none", "space", "home", "end", "escape"]
        if operation == "terminal":
            assert key != "none"
            if event == "begin":
                assert focused, "Terminal input lacks the recorded fresh focus proof"
                assert previous and previous["step"] == step and previous["operation"] == "focus" and previous["event"] == "observed"
                attempts += 1
        else:
            assert key == "none"
        assert type(row["terminal_attempts"]) is int and row["terminal_attempts"] == attempts
        if event == "observed":
            assert operation == "focus"
            assert type(row["eligible"]) is bool and type(row["focused"]) is bool
            focused = row["eligible"] and row["focused"]
        else:
            assert row["eligible"] is None and row["focused"] is None
        token = (step, operation, key)
        if event == "begin":
            stack.append(token)
            if operation == "tab":
                focused = False
        else:
            assert stack and stack.pop() == token, "Unfinished or mismatched await"
            if operation == "step":
                completed.append(step)
        previous = row
    assert not stack
    assert completed == STEPS + ["prefix", "cleanup"], "Only the complete shared prefix may pass"
    for step, key in [("pause-key", "space"), ("settings-open", "space"),
                      ("volume-home", "home"), ("volume-end", "end"),
                      ("settings-close", "escape")]:
        assert sum(r["step"] == step and r["operation"] == "terminal"
                   and r["event"] == "begin" and r["key"] == key for r in rows) == 1
    for step in ["volume-home", "volume-end"]:
        assert any(r["step"] == step and r["operation"] == "value-bound"
                   and r["event"] == "end" for r in rows)
    assert any(r["operation"] == "track-settlement" and r["event"] == "end" for r in rows)


def fixture():
    rows, attempts = [], 0
    def add(step, operation, event, key="none", flags=None):
        nonlocal attempts
        if operation == "terminal" and event == "begin":
            attempts += 1
        rows.append(dict(sequence=len(rows) + 1, elapsed_ms=len(rows), step=step,
                         operation=operation, event=event, key=key,
                         eligible=flags[0] if flags else None,
                         focused=flags[1] if flags else None, terminal_attempts=attempts))
    add("prefix", "step", "begin")
    for step in STEPS:
        add(step, "step", "begin")
        key = {"pause-key": "space", "settings-open": "space", "volume-home": "home",
               "volume-end": "end", "settings-close": "escape"}.get(step)
        if key:
            add(step, "focus", "begin")
            add(step, "focus", "observed", flags=(True, True))
            add(step, "terminal", "begin", key)
            add(step, "terminal", "end", key)
        operation = ("value-bound" if step in ["volume-home", "volume-end"] else
                     "track-settlement" if step == "track-choices" else
                     "menu-confirmation" if step in ["settings-open", "settings-close"] else None)
        if operation:
            add(step, operation, "begin")
            add(step, operation, "end")
        add(step, "step", "end")
    add("prefix", "step", "end")
    add("cleanup", "step", "begin")
    add("cleanup", "step", "end")
    return rows


def tests(output):
    output.mkdir(parents=True, exist_ok=True)
    here = Path(__file__).resolve().parent
    argv = sys.argv
    sys.argv = [str(here / "thread-limit-runner-test.py"), str(output / "security")]
    try:
        context = runpy.run_path(sys.argv[0])
    finally:
        sys.argv = argv
    docker, base, root = context["docker"], context["base"], context["root"]
    docker.write_text("""#!/usr/bin/env python3
import json,os,pathlib,shutil,sys
with open(os.environ['DOCKER_SENTINEL'],'a') as f: f.write(json.dumps(sys.argv[1:])+'\\n')
if sys.argv[1]=='create': print('fixture-container')
elif sys.argv[1]=='start':
 print('playback-stage: controls-prefix end')
 print('test result: ok. 1 passed; 0 failed; 0 ignored;' if os.environ['CASE']!='zero' else 'test result: ok. 0 passed; 0 failed; 0 ignored;')
 if os.environ['CASE']=='controls-leak': print('playback-stage: captures-controls begin')
 sys.exit(23 if os.environ['CASE']=='primary' else 0)
elif sys.argv[1]=='cp':
 source=pathlib.Path(os.environ['PAYLOAD']); dest=pathlib.Path(sys.argv[-1])
 for p in source.iterdir():
  if p.is_file(): shutil.copyfile(p,dest/p.name)
 if os.environ['CASE']=='manifest': (dest/'playback/manifest.json').write_text('{}')
elif sys.argv[1] not in ['build','rm']: raise SystemExit(90)
""")
    cases = ["valid", "missing", "pending", "failed", "unfocused", "raw", "cap",
             "time", "duplicate", "wrong-key", "partial", "manifest",
             "controls-leak", "zero", "primary"]
    for case in cases:
        payload, destination = output / f"{case}-payload", output / case
        payload.mkdir(exist_ok=True)
        destination.mkdir(exist_ok=True)
        rows = copy.deepcopy(fixture())
        terminal = next(r for r in rows if r["operation"] == "terminal")
        if case == "pending": rows = rows[:rows.index(terminal) + 1]
        if case == "failed": terminal["event"] = "failed"
        if case == "unfocused":
            next(r for r in rows if r["event"] == "observed")["focused"] = False
        if case == "raw": rows[0]["raw"] = "forbidden"
        if case == "cap": rows *= 257
        if case == "time": rows[1]["elapsed_ms"] = -1
        if case == "wrong-key": terminal["key"] = "end"
        if case == "partial": rows.pop()
        text = "".join(json.dumps(r) + "\n" for r in rows)
        if case == "duplicate": text = text.replace('"sequence": 1,', '"sequence": 1, "sequence": 1,', 1)
        if case != "missing": (payload / "playback-controls.jsonl").write_text(text)
        (destination / "playback-controls.jsonl").write_text("stale")
        env = {**base, "CASE": case, "PAYLOAD": str(payload),
               "SCREENROOM_E2E_JOURNEY": "playback", "SCREENROOM_E2E_PLAYBACK_PHASE": "controls",
               "SCREENROOM_PLAYBACK_DIAGNOSTICS": "1",
               "DOCKER_SENTINEL": str(output / f"{case}-docker.jsonl"),
               "BACKEND_SENTINEL": str(output / f"{case}-backend.log")}
        result = subprocess.run(["/bin/bash", str(here / "run.sh"), str(destination)],
                                env=env, capture_output=True, text=True)
        (output / f"{case}.log").write_text(result.stdout + result.stderr)
        assert result.returncode == (0 if case == "valid" else 23 if case == "primary" else 1), (case, result.returncode)
        if case == "missing": assert not (destination / "playback-controls.jsonl").exists()
        if case == "primary": assert (destination / "playback-controls.jsonl").read_text() == text
        calls = [json.loads(line) for line in (output / f"{case}-docker.jsonl").read_text().splitlines()]
        create, = [c for c in calls if c[0] == "create"]
        for flag, value in [("--pids-limit", "512"), ("--memory", "2g"), ("--cap-drop", "ALL"),
                            ("--security-opt", "no-new-privileges"), ("--network", "fixture-network")]:
            assert create[create.index(flag) + 1] == value
    print("15 control runner cases and four security cases passed; no actual Docker/backend/application.")


if __name__ == "__main__":
    if sys.argv[1] == "--validate":
        validate(Path(sys.argv[2]))
    elif sys.argv[1] == "--test":
        tests(Path(sys.argv[2]).resolve())
    else:
        raise SystemExit("Expected --validate or --test")
