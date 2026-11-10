instrumentation_serial=
instrumentation_package=
instrumentation_host_pid=
instrumentation_log=

stop_owned_instrumentation() {
  if [ -n "$instrumentation_host_pid" ]; then
    kill "$instrumentation_host_pid" 2>/dev/null || true
    for ((attempt = 0; attempt < 20; attempt++)); do
      kill -0 "$instrumentation_host_pid" 2>/dev/null || break
      sleep .1
    done
    kill -9 "$instrumentation_host_pid" 2>/dev/null || true
    wait "$instrumentation_host_pid" 2>/dev/null || true
    instrumentation_host_pid=
  fi
  if [ -n "$instrumentation_serial" ]; then
    local active
    active=$(command timeout --foreground --kill-after=2 10 "$ADB" -s "$instrumentation_serial" shell dumpsys activity instrumentation 2>/dev/null) || active=
    if grep -q '^INSTRUMENTATION_STATUS:' "$instrumentation_log" || printf '%s\n' "$active" | grep -Fq "$instrumentation_package.test/"; then
      for owned_package in "$instrumentation_package" "$instrumentation_package.test"; do
        command timeout --foreground --kill-after=2 10 "$ADB" -s "$instrumentation_serial" shell am force-stop "$owned_package" >/dev/null 2>&1 || true
      done
    fi
    instrumentation_serial=
    instrumentation_package=
  fi
}

run_owned_instrumentation() {
  local duration=$1 serial=$2 package=$3 log=$4 status
  shift 4
  local active
  active=$(command timeout --foreground --kill-after=2 15 "$ADB" -s "$serial" shell dumpsys activity instrumentation) || return $?
  if printf '%s\n' "$active" | grep -Fq "$package.test/"; then
    echo "Refusing to replace existing instrumentation for $package on $serial" >&2
    return 1
  fi
  : > "$log"
  instrumentation_serial=$serial
  instrumentation_package=$package
  instrumentation_log=$log
  command timeout --foreground --kill-after=5 "$duration" "$ADB" -s "$serial" shell am instrument -w -r "$@" \
    "$package.test/androidx.test.runner.AndroidJUnitRunner" >> "$log" &
  instrumentation_host_pid=$!
  if wait "$instrumentation_host_pid"; then status=0; else status=$?; fi
  instrumentation_host_pid=
  cat "$log"
  [ "$status" -eq 0 ] || return "$status"
  grep -Eq '^OK \(1 test\)' "$log" || return 1
  instrumentation_serial=
  instrumentation_package=
}
