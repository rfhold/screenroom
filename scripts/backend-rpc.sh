#!/usr/bin/env bash

backend_bounded() {
  local input=()
  if [[ $1 == --stdin ]]; then input=(--stdin); shift; fi
  local limit=$1
  shift
  if [[ -n ${backend_deadline-} ]]; then
    local left=$((backend_deadline-SECONDS))
    (( left > 0 )) || return 1
    if (( left < limit )); then limit=$left; fi
  fi
  python3 scripts/backend-process.py ${input[@]+"${input[@]}"} "$limit" "$@"
}

backend_native() {
  local origin=$1 user=$2 method=$3
  local headers=()
  if [[ -n $user ]]; then headers=(-H "x-screenroom-user-id: $user"); fi
  backend_bounded 17 2097152 grpcurl -plaintext -max-time 15 -import-path proto \
    -proto screenroom/catalog/v1/catalog.proto -proto screenroom/user/v1/user.proto \
    -proto screenroom/operator/v1/operator.proto ${headers[@]+"${headers[@]}"} \
    -d '{}' "${origin#http://}" "$method"
}

backend_web() (
  local origin=$1 user=$2 method=$3 output trace result
  output=$(mktemp "${TMPDIR:-/tmp}/screenroom-rpc-output.XXXXXXXX") || exit 1
  trap 'rm -f "$output" "${trace:-}"' EXIT
  trace=$(mktemp "${TMPDIR:-/tmp}/screenroom-rpc-trace.XXXXXXXX") || exit 1
  NO_PROXY='127.0.0.1' no_proxy='127.0.0.1' backend_bounded 17 2097152 \
    buf curl --schema . --protocol grpcweb --timeout 15s \
    --log-format text --verbose \
    -H 'Origin: http://127.0.0.1:4173' -H 'X-Grpc-Web: 1' \
    -H "X-Screenroom-User-Id: $user" "$origin/$method" >"$output" 2>"$trace" || exit 1
  LC_ALL=C awk '
    /^buf: < \(#1\) HTTP\// { responses++ }
    tolower($0) ~ /^buf: < \(#1\) access-control-allow-origin:/ {
      origins++; if ($5 == "http://127.0.0.1:4173" && NF == 5) allowed++
    }
    /^buf: } \(#1\) \[[0-9]+ bytes data\]$/ { sent += substr($4, 2) }
    /^buf: \{ \(#1\) \[[0-9]+ bytes data\]$/ { received += substr($4, 2) }
    END {
      exit !(responses == 1 && origins == 1 && allowed == 1 &&
        sent >= 5 && sent <= 65541 && received >= 5 && received <= 2097152)
    }' "$trace" || exit 1
  result=$(jq -ces 'select(length == 1 and (.[0] | type == "object")) | .[0]' "$output") || exit 1
  printf '%s\n' "$result"
)

backend_rpc_pair() {
  local origin=$1 native_user=$2 browser_user=$3 method=$4 native web
  native=$(backend_native "$origin" "$native_user" "$method") || return 1
  web=$(backend_web "$origin" "$browser_user" "$method") || return 1
  printf '%s\n%s\n' "$native" "$web" |
    jq -ecs 'if length == 2 and .[0] == .[1] then .[0] else error("native/grpc-web mismatch") end'
}
