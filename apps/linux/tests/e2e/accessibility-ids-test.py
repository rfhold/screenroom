from pathlib import Path
import os
import subprocess
import sys
import tempfile

root = Path(__file__).resolve().parents[4]
output = Path(sys.argv[1]).resolve()
output.mkdir(parents=True, exist_ok=True)
source = (root / "apps/linux/src/home.rs").read_text()
start = source.index("fn capture_item_description(")
opening = source.index("{", start)
depth = 0
for index in range(opening, len(source)):
    depth += (source[index] == "{") - (source[index] == "}")
    if depth == 0:
        function = source[start:index + 1]
        break
else:
    raise AssertionError("Production capture metadata gate is missing")

with tempfile.TemporaryDirectory(prefix="accessibility-gate-", dir=output) as temporary:
    work = Path(temporary)
    (work / "gate.rs").write_text(
        function
        + '\nfn main() { match capture_item_description("opaque:item") { Some(value) => println!("{value}"), None => println!("off") } }\n'
    )
    subprocess.run(["rustc", "--edition=2024", str(work / "gate.rs"), "-o", str(work / "gate")], check=True)
    for flag in [None, "", "0", "true", "other", "01", "1"]:
        env = dict(os.environ)
        env.pop("SCREENROOM_E2E_ACCESSIBILITY_IDS", None)
        if flag is not None:
            env["SCREENROOM_E2E_ACCESSIBILITY_IDS"] = flag
        result = subprocess.check_output([str(work / "gate")], env=env, text=True).strip()
        assert result == ("screenroom-item:opaque:item" if flag == "1" else "off"), (flag, result)
    print("Actual production gate: unset/empty/0/true/other/01 off; exactly 1 on.")
    assert 'if let Some(description) = capture_item_description(&item.id)' in source
    assert source.count("Property::Description(&description)") == 1

    stub = work / "screenroom-linux"
    stub.write_text('#!/usr/bin/env bash\nprintf "%s\\n" "${SCREENROOM_E2E_ACCESSIBILITY_IDS-unset}"\n')
    stub.chmod(0o755)
    launcher = root / "apps/linux/tests/e2e/app.sh"
    for journey, scenario, inherited, expected in [
        (None, None, None, "unset"),
        (None, None, "1", "unset"),
        ("browse", "happy_path", "1", "unset"),
        ("browse", "regression_search", "1", "unset"),
        ("playback", "happy_path", "1", "unset"),
        ("playback", "playback_showcase", None, "1"),
        ("playback", "playback_showcase", "0", "1"),
    ]:
        command = [
            "docker", "run", "--rm", "--network", "none", "--read-only",
            "--tmpfs", "/artifacts:mode=1777", "--cap-drop", "ALL",
            "--pids-limit", "16", "--memory", "128m",
            "--mount", f"type=bind,source={launcher},target=/capture.sh,readonly",
            "--mount", f"type=bind,source={stub},target=/usr/local/bin/screenroom-linux,readonly",
            "--entrypoint", "bash",
        ]
        for name, value in [
            ("SCREENROOM_E2E_JOURNEY", journey),
            ("E2E_SCENARIO", scenario),
            ("SCREENROOM_E2E_ACCESSIBILITY_IDS", inherited),
        ]:
            if value is not None:
                command += ["-e", f"{name}={value}"]
        command += ["screenroom-linux-e2e:local", "-c", "bash /capture.sh && cat /artifacts/app.log"]
        result = subprocess.check_output(command, text=True).strip()
        assert result == expected, (journey, scenario, inherited, result)
    print("Actual capture launcher: only playback:playback_showcase opts in; browse/default/other scenarios clear inherited flags. Stub app only; no GUI/backend/network.")
