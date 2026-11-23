#!/usr/bin/env python3
"""Offline checks: extracted diagnostic emitter and launcher with a stub executable."""
from pathlib import Path
import os
import subprocess
import sys

root = Path(__file__).resolve().parents[4]
output = Path(sys.argv[1]).resolve()
output.mkdir(parents=True, exist_ok=True)
source = (root / "apps/linux/src/playback.rs").read_text()
harness = (root / "apps/linux/tests/e2e/playback.rs").read_text()
readiness = harness.index('if phase == "readiness" {', harness.index("let readiness_result = ready"))
assert harness.index("return Ok(());", readiness) < harness.index('enter_stage(&mut stage, "captures-controls")')
assert 'if phase != "capture" {' in harness
assert harness.index('if phase != "capture" {') < harness.index("evidence.write(&item, &instance)")
assert "const A11Y" not in harness and ".set_default_timeout" not in harness
play_call = harness.index('click(&session, "Play").await?;')
assert harness.index("evidence.play_attempted = true;") < play_call
assert harness.index("evidence.play_return_confirmed = true;") > play_call
shutdown = harness.index('let shutdown = if phase == "readiness"')
assert harness.index("result?;", shutdown) < harness.index("cleanup??;", shutdown) < harness.index("shutdown?;", shutdown)
assert "stop_readiness(session, accessibility_failed).await" in harness
helpers = source[source.index("const DIAGNOSTIC_MARKERS:"):source.index("fn idle_diagnostics_enabled(")]
fixture = output / "emitter.rs"
fixture.write_text("use std::time::Instant;\n" + helpers + """
fn main() {
    for _ in 0..1000 { diagnostic_event("gtk.heartbeat"); }
}
""")
binary = output / "emitter"
subprocess.run(["rustc", "--edition=2024", str(fixture), "-o", str(binary)], check=True)
subprocess.run(["rustc", "--edition=2024", "--test", str(fixture), "-o", str(output / "emitter-tests")], check=True)
subprocess.run([str(output / "emitter-tests")], check=True)
base = {key: value for key, value in os.environ.items() if key not in (
    "SCREENROOM_PLAYBACK_DIAGNOSTICS", "SCREENROOM_E2E_PLAYBACK_PHASE", "SCREENROOM_E2E_JOURNEY")}
cases = [
    ({}, False),
    ({"SCREENROOM_PLAYBACK_DIAGNOSTICS": "1"}, False),
    ({"SCREENROOM_PLAYBACK_DIAGNOSTICS": "1", "SCREENROOM_E2E_PLAYBACK_PHASE": "capture", "SCREENROOM_E2E_JOURNEY": "playback"}, False),
    ({"SCREENROOM_PLAYBACK_DIAGNOSTICS": "0", "SCREENROOM_E2E_PLAYBACK_PHASE": "readiness", "SCREENROOM_E2E_JOURNEY": "playback"}, False),
    ({"SCREENROOM_PLAYBACK_DIAGNOSTICS": "1", "SCREENROOM_E2E_PLAYBACK_PHASE": "readiness", "SCREENROOM_E2E_JOURNEY": "browse"}, False),
    ({"SCREENROOM_PLAYBACK_DIAGNOSTICS": "1", "SCREENROOM_E2E_PLAYBACK_PHASE": "readiness", "SCREENROOM_E2E_JOURNEY": "playback"}, True),
]
stub = output / "application-stub.sh"
stub.write_text("""#!/bin/bash
set -eu
if [[ -e /artifacts/playback-owned.pid ]]; then
    [[ $(cat /artifacts/playback-owned.pid) == $$ ]]
    printf 'owned-pid-preserved\\n'
else
    printf 'no-pid-record\\n'
fi
""")
stub.chmod(0o755)
for index, (env, enabled) in enumerate(cases):
    result = subprocess.run([str(binary)], env={**base, **env}, check=True, capture_output=True, text=True)
    lines = result.stderr.splitlines()
    assert len(lines) == (256 if enabled else 0), (index, len(lines))
    for sequence, line in enumerate(lines, 1):
        prefix = f"playback-diagnostic sequence={sequence} elapsed_ms="
        assert line.startswith(prefix) and line.endswith(" marker=gtk.heartbeat")
        assert line[len(prefix):].split(" ", 1)[0].isdigit()
    command = [
        "docker", "run", "--rm", "--network", "none", "--read-only",
        "--tmpfs", "/artifacts:mode=1777", "--cap-drop", "ALL",
        "--entrypoint", "/bin/bash",
        "--mount", f"type=bind,src={root / 'apps/linux/tests/e2e/app.sh'},dst=/launcher.sh,readonly",
        "--mount", f"type=bind,src={stub},dst=/usr/local/bin/screenroom-linux,readonly",
    ]
    for key, value in env.items():
        command.extend(["-e", f"{key}={value}"])
    command.extend(["screenroom-linux-e2e:local", "-c", "bash /launcher.sh; cat /artifacts/app.log"])
    launched = subprocess.run(command, check=True, capture_output=True, text=True)
    assert launched.stdout.strip() == ("owned-pid-preserved" if enabled else "no-pid-record")
print("Six emitter/exec-launcher cases passed; default-off, readiness-only, 256-line cap, owned PID unchanged by exec.")
print("Readiness returns before the controls journey; only capture phase reaches manifest writing.")
