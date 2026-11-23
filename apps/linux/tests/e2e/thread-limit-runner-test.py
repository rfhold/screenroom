#!/usr/bin/env python3
"""Exercise the host runner with poison backend/Docker adapters, never a container."""
from pathlib import Path
import json
import os
import subprocess
import sys

root = Path(__file__).resolve().parents[4]
output = Path(sys.argv[1]).resolve()
output.mkdir(parents=True, exist_ok=True)
bin_dir = output / "bin"
bin_dir.mkdir(exist_ok=True)
docker = bin_dir / "docker"
docker.write_text("""#!/usr/bin/env python3
import json, os, sys
with open(os.environ['DOCKER_SENTINEL'], 'a') as target:
    target.write(json.dumps(sys.argv[1:]) + '\\n')
if sys.argv[1] == 'create':
    print('fixture-container')
elif sys.argv[1] == 'start':
    print('fixture intentionally stops before application launch')
    sys.exit(7)
elif sys.argv[1] not in ['build', 'cp', 'rm']:
    raise SystemExit('unexpected Docker operation')
""")
docker.chmod(0o755)
bash = bin_dir / "bash"
bash.write_text("""#!/bin/bash
if [[ $1 == */scripts/backend.sh ]]; then
    printf '%s\\n' "$2" >> "$BACKEND_SENTINEL"
    exit 0
fi
exec /bin/bash "$@"
""")
bash.chmod(0o755)
descriptor = output / "descriptor.json"
descriptor.write_text(json.dumps({
    "ready": True, "clientNetwork": "fixture-network", "clientUpstream": "fixture:50051",
    "instanceID": "11111111-1111-4111-8111-111111111111",
    "publicOrigin": "http://127.0.0.1:50052",
    "userID": "22222222-2222-4222-8222-222222222222",
}))
base = {key: value for key, value in os.environ.items() if not (
    key.startswith("SCREENROOM_") or key.startswith("E2E_"))}
base.update({
    "PATH": f"{bin_dir}:{base['PATH']}",
    "E2E_CATEGORY": "happy-path", "SCREENROOM_BACKEND_DESCRIPTOR": str(descriptor),
    "SCREENROOM_BACKEND_ATTACH": "1", "SCREENROOM_BACKEND_PROJECT": "screenroom-jellyfin-local",
    "SCREENROOM_API_ENDPOINT": "http://127.0.0.1:50052",
    "SCREENROOM_BACKEND_INSTANCE_ID": "11111111-1111-4111-8111-111111111111",
    "SCREENROOM_SHOWCASE_ITEM_ID": "fixture-item", "SCREENROOM_SHOWCASE_TITLE": "Nosferatu",
    "SCREENROOM_SHOWCASE_YEAR": "1922",
})
contracts = []
for name, journey, phase, diagnostics, expected in [
    ("browse", "browse", "capture", "0", "128"),
    ("playback", "playback", "capture", "0", "512"),
    ("navigation", "playback", "navigation", "0", "512"),
    ("readiness", "playback", "readiness", "1", "512"),
]:
    destination = output / name
    destination.mkdir(exist_ok=True)
    stale = destination / "playback"
    stale.mkdir(exist_ok=True)
    (stale / "manifest.json").write_text("{}")
    (stale / "stale-marker").write_text("must clear only for playback")
    protected = destination / "protected-unrelated"
    protected.write_text("preserve")
    sentinel = output / f"{name}-docker.jsonl"
    sentinel.write_text("")
    backend = output / f"{name}-backend.log"
    backend.write_text("")
    env = {**base, "SCREENROOM_E2E_JOURNEY": journey,
           "SCREENROOM_E2E_PLAYBACK_PHASE": phase, "SCREENROOM_PLAYBACK_DIAGNOSTICS": diagnostics,
           "DOCKER_SENTINEL": str(sentinel), "BACKEND_SENTINEL": str(backend)}
    result = subprocess.run(["/bin/bash", str(root / "apps/linux/tests/e2e/run.sh"), str(destination)],
                            env=env, capture_output=True, text=True)
    (output / f"{name}.log").write_text(result.stdout + result.stderr)
    assert result.returncode == 7, (name, result.returncode, result.stderr)
    calls = [json.loads(line) for line in sentinel.read_text().splitlines()]
    creates = [args for args in calls if args[0] == "create"]
    assert len(creates) == 1
    args = creates[0]
    assert args[args.index("--pids-limit") + 1] == expected
    assert 0 < int(expected) <= 512
    assert args[args.index("--memory") + 1] == "2g"
    assert args[args.index("--cap-drop") + 1] == "ALL"
    assert args[args.index("--security-opt") + 1] == "no-new-privileges"
    assert args[args.index("--network") + 1] == "fixture-network"
    assert args[-1] == "screenroom-linux-e2e:local"
    if journey == "playback":
        assert not (stale / "manifest.json").exists() and not (stale / "stale-marker").exists()
    else:
        assert (stale / "stale-marker").read_text() == "must clear only for playback"
    assert protected.read_text() == "preserve"
    contracts.append(args)
    print(f"{name}: one create; PID limit {expected}; 2g/network/capability/security contract preserved; intentional failure nonzero")
for args in contracts[1:]:
    def security(arguments):
        return [arguments[arguments.index(flag):arguments.index(flag) + 2]
                for flag in ["--network", "--cap-drop", "--security-opt", "--memory"]]
    assert security(args) == security(contracts[0])
print("No real Docker/backend/application invocation; playback directories fresh; unrelated paths preserved.")
