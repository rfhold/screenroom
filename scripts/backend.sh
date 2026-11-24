#!/usr/bin/env bash
set -euo pipefail
cd "$(dirname "$0")/.."
if [[ $# == 2 && $1 == status && $2 == --live ]]; then
  exec bash scripts/backend-status.sh
fi
source scripts/backend-rpc.sh

fail() { printf 'backend: %s\n' "$1" >&2; exit 1; }
file() { python3 scripts/backend-files.py "$@"; }
[[ ! ${SCREENROOM_BACKEND_PROFILE+x} ]] || fail 'backend profiles are retired; use the ordinary local installation'
project=${SCREENROOM_BACKEND_PROJECT-screenroom-local}
port=${SCREENROOM_BACKEND_PORT-50051}
[[ $project =~ ^screenroom-[a-z0-9]+(-[a-z0-9]+)*$ ]] || fail 'invalid backend project'
[[ $port =~ ^[0-9]{1,5}$ ]] && ((10#$port >= 1024 && 10#$port <= 65535)) ||
  fail 'invalid explicit backend port'
origin="http://127.0.0.1:$port"
[[ ${SCREENROOM_API_ENDPOINT-$origin} == "$origin" && ${SCREENROOM_E2E_API_ENDPOINT-$origin} == "$origin" ]] ||
  fail "backend endpoint must equal $origin"

runtime="$PWD/build/backend-runtime/$project"
export SCREENROOM_API_ENDPOINT="$origin" SCREENROOM_BACKEND_PORT="$port"
export SCREENROOM_BACKEND_ENV="$runtime/runtime.env" SCREENROOM_BACKEND_RUNTIME="$runtime"
compose=(docker compose --env-file "$SCREENROOM_BACKEND_ENV" -p "$project" -f "$PWD/compose.yaml")
owner= command= locked=no
if [[ ${1-} == --locked ]]; then locked=yes; shift; fi
while (($#)); do
  case $1 in
    --owner-file) [[ $# -ge 2 && -n $2 && -z $owner ]] || fail 'expected one owner file'; owner=$2; shift ;;
    initialize|up|wait|status|down|drain|activate-model)
      [[ -z $command ]] || fail 'expected one action'
      command=$1 ;;
    *) fail 'usage: bash scripts/backend.sh ACTION [--owner-file PATH]' ;;
  esac
  shift
done
[[ -n $command ]] || fail 'missing action'
[[ ! -L $runtime ]] || fail 'runtime must not be a symlink'

case $command in
  initialize|up|drain|activate-model)
    if [[ ${SCREENROOM_EMBEDDING_BASE_URL+x}${SCREENROOM_EMBEDDING_MODEL+x}${SCREENROOM_EMBEDDING_DIMENSIONS+x}${SCREENROOM_EMBEDDING_REVISION+x}${SCREENROOM_EMBEDDING_API_KEY+x} ]]; then
      base=${SCREENROOM_EMBEDDING_BASE_URL-} model=${SCREENROOM_EMBEDDING_MODEL-} revision=${SCREENROOM_EMBEDDING_REVISION-}
      export LC_ALL=C
      [[ $base =~ ^https?://[^/@:\?#[:space:]]+(:[0-9]+)?(/[^?\#[:space:]]*)?$ ||
         $base =~ ^https?://\[[[:xdigit:]:.]+\](:[0-9]+)?(/[^?\#[:space:]]*)?$ ]] &&
      [[ ${SCREENROOM_EMBEDDING_DIMENSIONS-} == 1024 && -n $model && -n $revision &&
         ${#model} -le 200 && ${#revision} -le 200 ]] ||
        fail 'invalid optional embedding configuration; provide URL, model, 1024 dimensions and revision'
    fi ;;
esac
if [[ $command != status ]]; then
  mode=exclusive
  [[ $command != wait ]] || mode=shared
  if [[ $locked == no ]]; then
    args=("$command")
    if [[ -n $owner ]]; then args+=(--owner-file "$owner"); fi
    exec python3 scripts/backend-files.py lock "$runtime" "$mode" scripts/backend.sh "${args[@]}"
  fi
  file verify-lock "$runtime" "$mode"
fi

bounded() { backend_bounded 45 2097152 "$@" 2>/dev/null; }
docker_compose() { "${compose[@]}" "$@" >&2; }
garage() { bounded "${compose[@]}" exec -T garage /garage "$@"; }
owned() { [[ -f $owner && ! -L $owner && -O $owner && $(file read "$owner") == "$project" ]]; }

select_user() {
  local users=$1 selected= source
  for source in "$runtime/local-user.json" "$runtime/descriptor.json"; do
    if [[ -e $source || -L $source ]]; then
      selected=$(file read "$source" | jq -er '.userID | strings') || return 1
      [[ -z $selected ]] || break
    fi
  done
  if [[ -z $selected ]]; then
    selected=$(file uuid "$(jq -er '.[0].id' <<< "$users")") || return 1
  fi
  jq -e --arg id "$selected" 'any(.[]; (.id | ascii_downcase | gsub("[{}-]";"")) == ($id | ascii_downcase | gsub("-";"")))' \
    <<< "$users" >/dev/null || return 1
  if [[ ! -e $runtime/local-user.json ]]; then
    jq -nc --arg user "$selected" '{userID:$user}' | file create "$runtime/local-user.json" 644 || return 1
  fi
  [[ $(file read "$runtime/local-user.json" | jq -er '.userID') == "$selected" ]] || return 1
  printf '%s\n' "$selected"
}

initialize_files() {
  local name existing= id
  users=
  for name in runtime.env identity.json users.json descriptor.json local-user.json; do
    [[ ! -L $runtime/$name ]] || fail 'runtime files must not be symlinks'
  done
  if [[ -e $runtime/users.json ]]; then users=$(file project-users "$runtime/users.json") || exit 1; fi
  chmod 700 "$runtime"
  if [[ ! -e $runtime/runtime.env || ! -e $runtime/identity.json || ! -e $runtime/users.json ]]; then
    existing=$(bounded docker volume ls -q --filter "label=com.docker.compose.project=$project") ||
      fail 'cannot verify the selected project volumes'
  fi
  if [[ ! -e $runtime/runtime.env ]]; then
    [[ -z $existing ]] || fail 'persistent volumes exist but runtime keys are missing; restore runtime.env, never rotate implicitly'
    file create-env "$runtime/runtime.env"
  fi
  file read-private "$runtime/runtime.env" >/dev/null
  if [[ ! -e $runtime/identity.json ]]; then
    [[ -z $existing ]] || fail 'persistent volumes exist but instance identity is missing; restore identity.json'
    id=$(file uuid)
    jq -nc --arg id "$id" --arg project "$project" --arg origin "$origin" \
      '{instanceID:$id,project:$project,bootstrapRevision:"installation-v1",publicOrigin:$origin,clientNetwork:($project+"_client")}' |
      file create "$runtime/identity.json" 644
  fi
  identity=$(file read "$runtime/identity.json")
  id=$(jq -er --arg p "$project" --arg o "$origin" \
    'select(.project==$p and .publicOrigin==$o and .clientNetwork==($p+"_client")) | .instanceID | strings' \
    <<< "$identity") || fail 'persistent instance origin mismatch'
  file uuid "$id" >/dev/null
  if [[ -z $users ]]; then
    [[ -z $existing ]] || fail 'persistent volumes exist but user identity is missing; restore users.json'
    id=$(file uuid)
    jq -nc --arg id "$id" '[{id:$id,display_name:"Local user"}]' | file create "$runtime/users.json" 644
    users=$(file project-users "$runtime/users.json")
  fi
  selected=$(select_user "$users") || fail 'invalid installation user selector'
}

bootstrap_garage() {
  local end=$((SECONDS+90)) status layout node name key value env role
  local catalog_key= catalog_secret= worker_key= worker_secret=
  until status=$(garage status); do
    (( SECONDS < end )) || fail 'Garage did not start'
    sleep 1
  done
  layout=$(garage layout show)
  if [[ $layout == *'Current cluster layout version: 0'* ]]; then
    node=$(awk '$1 ~ /^[a-f0-9]+$/ && length($1) >= 16 { print $1; exit }' <<< "$status")
    [[ -n $node ]] || fail 'cannot find Garage node identity'
    garage layout assign -z local -c 1G "$node" >/dev/null
    garage layout apply --version 1 >/dev/null
  fi
  env=$(file read-private "$runtime/runtime.env")
  while IFS='=' read -r name value; do
    case $name in
      CATALOG_S3_KEY) catalog_key=$value ;; CATALOG_S3_SECRET) catalog_secret=$value ;;
      WORKER_S3_KEY) worker_key=$value ;; WORKER_S3_SECRET) worker_secret=$value ;;
    esac
  done <<< "$env"
  garage bucket info screenroom-artwork >/dev/null || garage bucket create screenroom-artwork >/dev/null
  for role in catalog worker; do
    local access=(--read)
    if [[ $role == catalog ]]; then key=$catalog_key; value=$catalog_secret
    else key=$worker_key; value=$worker_secret; access+=(--write); fi
    [[ $key =~ ^GK[[:xdigit:]]{24}$ && $value =~ ^[[:xdigit:]]{64}$ ]] ||
      fail 'invalid local artwork credentials'
    garage key info "$key" >/dev/null ||
      garage key import --yes -n "screenroom-$role" "$key" "$value" >/dev/null
    garage bucket allow screenroom-artwork --key "$key" "${access[@]}" >/dev/null
  done
}

setup_directory() {
  local service=$1 directory=$2
  bounded "${compose[@]}" run --rm --no-deps --user 0:0 --entrypoint /bin/sh "$service" -ec '
    d=$1
    test -d "$d" && test ! -L "$d"
    if [ "$(stat -c "%u:%g:%a" "$d")" = 0:0:755 ] && [ -z "$(ls -A "$d")" ]; then
      chown 10001:10001 "$d"
      chmod 700 "$d"
    fi
    test "$(stat -c "%u:%g:%a" "$d")" = 10001:10001:700
  ' screenroom "$directory" >/dev/null
}

edge_identity() {
  backend_bounded 6 1048576 curl -fsS --noproxy '*' --max-time 5 "$origin/__screenroom_instance"
}

wait_once() {
  local selected expected actual response users
  users=$(file project-users "$runtime/users.json") || return 1
  selected=$(select_user "$users") || return 1
  expected=$(file read "$runtime/identity.json") || return 1
  actual=$(edge_identity) || return 1
  jq -en --argjson a "$actual" --argjson b "$expected" '$a == $b' >/dev/null || return 1
  backend_rpc_pair "$origin" '' "$selected" screenroom.catalog.v1.CatalogService/GetCatalogInfo >/dev/null || return 1
  response=$(backend_rpc_pair "$origin" "$selected" "$selected" screenroom.user.v1.UserService/GetCurrentUser) || return 1
  jq -e --arg user "$selected" '.user.id == $user' <<< "$response" >/dev/null || return 1
  backend_bounded 5 8192 "${compose[@]}" exec -T playback-gateway curl -fsS --max-time 3 \
    http://127.0.0.1:50054/ready >/dev/null || return 1
  backend_native "$origin" '' screenroom.operator.v1.OperatorService/Health >/dev/null || return 1
  jq -e --arg user "$selected" '. + {userID:$user,clientUpstream:"edge:50051",ready:true,checks:{native:true,grpcWeb:true,identity:true,playback:true}}' \
    <<< "$expected" | file replace "$runtime/descriptor.json" 644 || return 1
  printf '%s\n' "$runtime/descriptor.json"
}

wait_backend() {
  local seconds=${SCREENROOM_BACKEND_WAIT_SECONDS-120} backend_deadline
  [[ $seconds =~ ^[0-9]{1,3}$ ]] && ((10#$seconds >= 1 && 10#$seconds <= 900)) ||
    fail 'wait must be between 1 and 900 seconds'
  backend_deadline=$((SECONDS+10#$seconds))
  until wait_once 2>/dev/null; do
    (( SECONDS < backend_deadline )) || fail 'readiness timed out; inspect private service diagnostics'
    sleep 1
  done
}

case $command in
  initialize) initialize_files ;;
  up)
    build=${SCREENROOM_BACKEND_BUILD-1}
    [[ $build == 0 || $build == 1 ]] || fail 'SCREENROOM_BACKEND_BUILD must be 0 or 1'
    if [[ -n $owner ]]; then
      running=$(bounded docker ps -q --filter "label=com.docker.compose.project=$project")
      [[ -z $running ]] || fail 'caller-owned project containers already running; use explicit attach mode'
      [[ ! -e $owner && ! -L $owner ]] || owned || fail 'teardown owner mismatch'
    fi
    initialize_files
    if [[ -n $owner && ! -e $owner ]]; then printf '%s' "$project" | file create "$owner" 600; fi
    docker_compose up -d --wait postgres garage
    docker_compose exec -T postgres psql -v ON_ERROR_STOP=1 -U screenroom -d screenroom -f /docker-entrypoint-initdb.d/10-runtime.sql
    bootstrap_garage
    if [[ $build == 1 ]]; then docker_compose build catalog; fi
    setup_directory index-operator /run/screenroom/credentials
    setup_directory playback-gateway /run/screenroom/playback
    printf '%s' "$users" | docker_compose run --rm -T --no-deps user screenroom-user-init --create-missing -
    backend_bounded --stdin 45 1048576 "${compose[@]}" exec -T postgres psql \
      -XAt -v ON_ERROR_STOP=1 -U screenroom -d screenroom >/dev/null 2>&1 <<'SQL' ||
      fail 'operator role provisioning did not complete'
DO $$ BEGIN
  IF NOT EXISTS (SELECT FROM pg_roles WHERE rolname='operator_runtime') THEN
    CREATE ROLE operator_runtime LOGIN INHERIT PASSWORD 'operator-local-only' IN ROLE screenroom_operator;
  END IF;
END $$;
SQL
    docker_compose up -d user catalog worker playback-gateway index-operator edge
    wait_backend ;;
  wait) wait_backend ;;
  status)
    descriptor=$(file read "$runtime/descriptor.json")
    expected=$(file read "$runtime/identity.json")
    actual=$(edge_identity)
    users=$(file project-users "$runtime/users.json")
    selected=$(select_user "$users") || fail 'invalid installation user selector'
    jq -en --argjson actual "$actual" --argjson expected "$expected" \
      --argjson d "$descriptor" --arg user "$selected" \
      'if $actual==$expected and $d.ready==true and $d.userID==$user and
          ($expected | to_entries | all(. as $kv | $d[$kv.key] == $kv.value))
       then $d else error("instance identity or user selector mismatch") end' ;;
  down)
    if [[ -n $owner ]]; then owned || fail 'teardown owner mismatch'; fi
    docker_compose down --timeout 75
    rm -f "$runtime/descriptor.json" ;;
  drain|activate-model)
    name="$project-operation-$(file uuid)"
    stop_worker() { backend_bounded 30 1048576 docker stop --time 15 "$name" >/dev/null 2>&1 || :; }
    interrupt_worker() {
      local pid=${worker_pid:-${!:-}}
      if [[ -n $pid ]]; then
        kill -TERM "$pid" 2>/dev/null || :
        wait "$pid" || :
      fi
      exit "$1"
    }
    trap stop_worker EXIT
    trap 'interrupt_worker 130' INT
    trap 'interrupt_worker 143' TERM
    python3 scripts/backend-process.py 1800 1048576 "${compose[@]}" run --rm --no-deps --name "$name" \
      worker screenroom-index-worker "$command" >/dev/null 2>&1 &
    worker_pid=$!
    wait "$worker_pid" || fail 'worker operation did not complete'
    trap - EXIT INT TERM ;;
esac
