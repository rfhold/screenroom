#!/usr/bin/env bash
set -euo pipefail
cd "$(dirname "$0")/.."
source scripts/backend-rpc.sh

fail() { printf 'backend contract: %s\n' "$1" >&2; exit 1; }
if [[ $# == 0 ]]; then
  descriptor=$(bash scripts/backend.sh status) || fail 'no verified local attachment'
  origin=$(jq -er '.publicOrigin' <<< "$descriptor")
  user=$(jq -er '.userID' <<< "$descriptor")
elif [[ $# == 2 ]]; then
  origin=$1 user=$2
else
  fail 'usage: bash scripts/backend-contract.sh [LOOPBACK_ORIGIN USER_ID]'
fi
[[ $origin =~ ^http://127\.0\.0\.1:([0-9]{1,5})$ ]] || fail 'expected a loopback origin'
port=${BASH_REMATCH[1]}
(( 10#$port >= 1024 && 10#$port <= 65535 )) || fail 'invalid loopback port'
[[ $user =~ ^[[:xdigit:]]{8}(-[[:xdigit:]]{4}){3}-[[:xdigit:]]{12}$ ]] || fail 'invalid user selector'
backend_deadline=$((SECONDS+120))

for method in screenroom.catalog.v1.BrowseService/GetHome \
  screenroom.catalog.v1.CatalogService/GetCatalogInfo \
  screenroom.catalog.v1.BrowseService/ListLibraries \
  screenroom.catalog.v1.SearchService/GetSearchCapabilities \
  screenroom.user.v1.UserService/GetCurrentUser; do
  response=$(backend_rpc_pair "$origin" "$user" "$user" "$method") || fail "RPC check failed: $method"
done
jq -e --arg user "$user" '.user.id == $user' <<< "$response" >/dev/null ||
  fail 'installation user mismatch'

for path in /screenroom.catalog.operator.v1.IngestionService/GetPolicy \
  /screenroom.user.operator.v1.JellyfinUserService/SetLink \
  /screenroom.user.internal.v1.PlaybackStateService/GetContinueWatching /ready; do
  code=$(printf '\0\0\0\0\0' | backend_bounded --stdin 6 8192 curl --silent --show-error --noproxy '*' --max-time 5 \
    --request POST --header 'Content-Type: application/grpc-web+proto' \
    --data-binary @- \
    --output /dev/null --write-out '%{http_code}' "$origin$path") || fail 'private-route check failed'
  [[ $code == 404 ]] || fail "private route did not return 404: $path"
done
jq -nc --arg user "$user" '{nativeGrpc:true,grpcWeb:true,privateRoutesAbsent:true,userID:$user}'
