#!/usr/bin/env bash
unset SCREENROOM_E2E_ACCESSIBILITY_IDS
if [[ ${SCREENROOM_E2E_JOURNEY:-browse}:${E2E_SCENARIO:-happy_path} == playback:playback_showcase ]]; then
    export SCREENROOM_E2E_ACCESSIBILITY_IDS=1
fi
if [[ ${SCREENROOM_PLAYBACK_DIAGNOSTICS:-}:${SCREENROOM_E2E_PLAYBACK_PHASE:-}:${SCREENROOM_E2E_JOURNEY:-} == 1:readiness:playback ]]; then
    printf '%s\n' "$$" > /artifacts/playback-owned.pid
fi
exec /usr/local/bin/screenroom-linux "$@" > /artifacts/app.log 2>&1
