import os
from pathlib import Path
import signal
import subprocess
import tempfile
import time
import unittest


HELPER = Path(__file__).with_name("instrumentation.sh")


class InstrumentationOwnershipTest(unittest.TestCase):
    def run_case(self, mode, interrupt=None):
        with tempfile.TemporaryDirectory(dir=Path(__file__).resolve().parents[1] / "build") as directory:
            root = Path(directory)
            adb = root / "adb"
            adb.write_text("""#!/usr/bin/env bash
printf '%s\\n' "$*" >> "$CALLS"
case "$*" in
  *dumpsys*) [ "$MODE" != existing ] || echo 'tv.screenroom.test/runner'; exit 0 ;;
  *force-stop*) exit 0 ;;
  *instrument*)
    [ "$MODE" != unstarted ] || exit 1
    echo 'INSTRUMENTATION_STATUS: id=AndroidJUnitRunner'
    touch "$STARTED"
    if [ "$MODE" = success ]; then echo 'OK (1 test)'; exit 0; fi
    [ "$MODE" != failure ] || exit 1
    exec sleep 20 ;;
esac
""")
            adb.chmod(0o755)
            env = dict(os.environ, ADB=str(adb), CALLS=str(root / "calls"), STARTED=str(root / "started"), MODE=mode)
            (root / "log").write_text("INSTRUMENTATION_STATUS: stale prior run\n")
            script = f'''set -euo pipefail
source '{HELPER}'
trap stop_owned_instrumentation EXIT
trap 'exit 130' INT
trap 'exit 143' TERM
run_owned_instrumentation {'20' if interrupt else '.2'} emulator-5554 tv.screenroom '{root / "log"}' -e class OwnTest
'''
            process = subprocess.Popen(["bash", "-c", script], env=env, stdout=subprocess.PIPE, stderr=subprocess.PIPE)
            if interrupt:
                deadline = time.monotonic() + 5
                while not (root / "started").exists() and time.monotonic() < deadline:
                    time.sleep(.01)
                self.assertTrue((root / "started").exists())
                process.send_signal(interrupt)
            output = process.communicate(timeout=10)
            calls = (root / "calls").read_text().splitlines()
            stops = [call for call in calls if "force-stop" in call]
            if mode in ("success", "unstarted", "existing"):
                self.assertEqual([], stops, output)
            else:
                self.assertEqual([
                    "-s emulator-5554 shell am force-stop tv.screenroom",
                    "-s emulator-5554 shell am force-stop tv.screenroom.test",
                ], stops, output)
            self.assertFalse(any("reverse" in call or "emu " in call or "backend" in call or "5556" in call for call in calls))
            self.assertEqual(mode == "success", process.returncode == 0, output)

    def test_timeout(self): self.run_case("timeout")
    def test_failure(self): self.run_case("failure")
    def test_success(self): self.run_case("success")
    def test_unstarted_app_is_not_stopped(self): self.run_case("unstarted")
    def test_existing_instrumentation_is_not_replaced(self): self.run_case("existing")
    def test_term(self): self.run_case("timeout", signal.SIGTERM)
    def test_int(self): self.run_case("timeout", signal.SIGINT)


if __name__ == "__main__":
    unittest.main()
