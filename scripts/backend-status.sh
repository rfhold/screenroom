#!/usr/bin/env bash
set -euo pipefail
cd "$(dirname "$0")/.."

fail() { printf 'backend status: %s\n' "$1" >&2; exit 1; }
[[ ! ${SCREENROOM_BACKEND_PROFILE+x} ]] || fail 'backend profiles are retired'
project=${SCREENROOM_BACKEND_PROJECT:-screenroom-local}
port=${SCREENROOM_BACKEND_PORT:-50051}
[[ $project =~ ^screenroom-[a-z0-9]+(-[a-z0-9]+)*$ ]] || fail 'invalid backend project'
[[ $port =~ ^[0-9]{1,5}$ ]] && (( 10#$port >= 1024 && 10#$port <= 65535 )) ||
  fail 'invalid explicit backend port'
origin="http://127.0.0.1:$port"
[[ ${SCREENROOM_API_ENDPOINT-$origin} == "$origin" && ${SCREENROOM_E2E_API_ENDPOINT-$origin} == "$origin" ]] ||
  fail 'backend endpoint must match the selected loopback port'

runtime="$PWD/build/backend-runtime/$project"
[[ ! -L $runtime ]] || fail 'runtime must not be a symlink'
export SCREENROOM_BACKEND_RUNTIME="$runtime" SCREENROOM_BACKEND_PORT="$port" SCREENROOM_API_ENDPOINT="$origin"
bounded() { python3 scripts/backend-process.py 6 2097152 "$@"; }
compose=(docker compose --env-file "$runtime/runtime.env" -p "$project" -f "$PWD/compose.yaml")
compose_read() { bounded "${compose[@]}" "$@" 2>/dev/null; }
rpc() {
  bounded grpcurl -plaintext -max-time 5 -import-path proto \
    -proto screenroom/catalog/v1/catalog.proto -proto screenroom/operator/v1/operator.proto \
    -d "$2" "127.0.0.1:$port" "$1" 2>/dev/null
}
json_file() {
  [[ -f $1 && ! -L $1 ]] && head -c 1048577 "$1" | jq -ce 'select(type == "object")'
}

attachment=unknown edge=unknown worker=unknown catalog=unknown playback=unknown queue=null
if descriptor=$(json_file "$runtime/descriptor.json" 2>/dev/null) &&
   identity=$(json_file "$runtime/identity.json" 2>/dev/null) &&
   selector=$(json_file "$runtime/local-user.json" 2>/dev/null) &&
   jq -en --argjson d "$descriptor" --argjson i "$identity" --argjson s "$selector" \
     '$d.ready == true and ($i | length > 0) and ($i == ($d | with_entries(select(.key as $k | $i | has($k))))) and ($d.userID | type == "string") and $d.userID == $s.userID' >/dev/null; then
  attachment=last_verified
  if current=$(bounded curl -fsS --noproxy '*' --max-time 5 "$origin/__screenroom_instance" 2>/dev/null) &&
     jq -en --argjson current "$current" --argjson expected "$identity" '$current == $expected' >/dev/null 2>&1; then
    edge=matched
  fi
fi

if services=$(compose_read ps --all --services) && running=$(compose_read ps --status running --services); then
  if grep -Fxq worker <<< "$running"; then worker=process_running
  elif grep -Fxq worker <<< "$services"; then worker=stopped
  else worker=missing
  fi
fi
if compose_read exec -T playback-gateway curl -fsS --max-time 3 http://127.0.0.1:50054/ready >/dev/null; then
  playback=ready
fi
if [[ $edge == matched ]]; then
  if rpc screenroom.catalog.v1.CatalogService/GetCatalogInfo '{}' >/dev/null; then catalog=ready; fi
  if observation=$(rpc screenroom.operator.v1.OperatorService/ObserveQueue '{"payloadJson":"e30="}') &&
     counts=$(jq -ce '.payloadJson | @base64d | fromjson | .queue | select(type == "object")
                       | with_entries(select(.value | type == "number" or type == "null"))' <<< "$observation" 2>/dev/null); then
    queue=$counts
  fi
fi
jq -nc --arg attachment "$attachment" --arg edge "$edge" --arg catalog "$catalog" \
  --arg playback "$playback" --arg worker "$worker" --argjson queue "$queue" \
  '{attachment:$attachment, current:{edge:$edge,catalog:$catalog,playback:$playback,
    worker:{process:$worker,loop:"unknown",degraded:($worker=="stopped" or $worker=="missing")},queue:$queue}}'
