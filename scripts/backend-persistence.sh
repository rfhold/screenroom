#!/usr/bin/env bash
set -euo pipefail

fail() { printf 'backend persistence: %s\n' "$1" >&2; exit 1; }

[[ $# == 1 ]] || fail 'usage: bash scripts/backend-persistence.sh OWNER_FILE'
owner=$1
[[ $owner == /* ]] || owner="$PWD/$owner"
cd "$(dirname "$0")/.."
project=${SCREENROOM_BACKEND_PROJECT:-}
[[ $project =~ ^screenroom-[a-z0-9]+(-[a-z0-9]+)*$ && $project != screenroom-local ]] ||
  fail 'select a dedicated disposable backend project with SCREENROOM_BACKEND_PROJECT'

owned() {
  [[ -f $owner && ! -L $owner && -O $owner && $(cat "$owner") == "$project" ]]
}
owned || fail 'teardown owner mismatch'

runtime="$PWD/build/backend-runtime/$project"
compose=(docker compose --env-file "$runtime/runtime.env" -p "$project" -f "$PWD/compose.yaml")
psql() {
  SCREENROOM_BACKEND_RUNTIME="$runtime" python3 scripts/backend-process.py --stdin 45 1048576 \
    "${compose[@]}" exec -T -e 'PGOPTIONS=-c statement_timeout=30000' \
    postgres psql -XqAt -v ON_ERROR_STOP=1 -U screenroom -d screenroom "$@"
}

snapshot() {
  psql <<'SQL'
SELECT jsonb_build_object(
  'users', (SELECT md5(coalesce(string_agg(row_to_json(t)::text, '' ORDER BY id), '')) FROM identity.users t),
  'links', (SELECT md5(coalesce(string_agg(row_to_json(t)::text, '' ORDER BY user_id,source_id), '')) FROM identity.jellyfin_user_links t),
  'sources', (SELECT md5(coalesce(string_agg(row_to_json(t)::text, '' ORDER BY id), '')) FROM catalog.sources t),
  'items', (SELECT md5(coalesce(string_agg(row_to_json(t)::text, '' ORDER BY id), '')) FROM catalog.items t),
  'source_items', (SELECT md5(coalesce(string_agg(row_to_json(t)::text, '' ORDER BY id), '')) FROM catalog.source_items t),
  'policies', (SELECT md5(coalesce(string_agg(row_to_json(t)::text, '' ORDER BY id), '')) FROM ingest.policies t),
  'objects', (SELECT md5(coalesce(string_agg(row_to_json(t)::text, '' ORDER BY row_to_json(t)::text), '')) FROM media.objects t),
  'artwork', (SELECT md5(coalesce(string_agg(row_to_json(t)::text, '' ORDER BY row_to_json(t)::text), '')) FROM media.artwork t),
  'variants', (SELECT md5(coalesce(string_agg(row_to_json(t)::text, '' ORDER BY row_to_json(t)::text), '')) FROM media.variants t),
  'documents', (SELECT md5(coalesce(string_agg(row_to_json(t)::text, '' ORDER BY row_to_json(t)::text), '')) FROM search.documents t),
  'embedding_models', (SELECT md5(coalesce(string_agg(row_to_json(t)::text, '' ORDER BY id), '')) FROM search.embedding_models t),
  'embeddings', (SELECT md5(coalesce(string_agg(row_to_json(t)::text, '' ORDER BY row_to_json(t)::text), '')) FROM search.embeddings t)
);
SQL
}

descriptor=$(bash scripts/backend.sh wait)
[[ $descriptor == "$runtime/descriptor.json" && -f $descriptor && ! -L $descriptor ]] ||
  fail 'attachment descriptor mismatch'
[[ -d $runtime && ! -L $runtime && -O $runtime ]] || fail 'runtime directory must be owned'
selector=$(jq -er '.userID | select(type == "string")' "$descriptor")
[[ $selector =~ ^[[:xdigit:]]{8}(-[[:xdigit:]]{4}){3}-[[:xdigit:]]{12}$ ]] || fail 'invalid user selector'
instance=$(jq -er '.instanceID | select(type == "string")' "$descriptor")
files() { shasum -a 256 "$runtime/identity.json" "$runtime/users.json" "$runtime/local-user.json"; }
bootstrap=$(files)
original=$(snapshot)
user=$(psql -v selector="$selector" <<'SQL'
SELECT jsonb_build_object('display_name',display_name,'updated_at',updated_at)
FROM identity.users WHERE id=:'selector'::uuid;
SQL
)
jq -e '.display_name != null and .updated_at != null' >/dev/null <<< "$user"
owned || fail 'teardown owner mismatch'
umask 077
recovery=$(mktemp "$runtime/operator-restore-XXXXXX")
marker="Persistence check $(openssl rand -hex 8)"
jq -nc --arg id "$selector" --arg marker "$marker" --argjson original "$user" \
  '{userID:$id,expectedDisplayName:$marker,original:$original}' > "$recovery"

restore() {
  owned || return 1
  psql -v selector="$selector" -v marker="$marker" -v original="$user" <<'SQL'
UPDATE identity.users SET display_name=(:'original'::jsonb)->>'display_name',
       updated_at=((:'original'::jsonb)->>'updated_at')::timestamptz
WHERE id=:'selector'::uuid AND display_name=:'marker';
SQL
}

cleanup() {
  local status=$?
  trap - EXIT
  if [[ $restored == yes ]] || { restore >/dev/null && [[ $(snapshot) == "$original" ]]; }; then
    rm "$recovery"
  else
    printf 'backend persistence: explicit restore required; preserve %s\n' "$recovery" >&2
    status=1
  fi
  exit "$status"
}
restored=no
trap cleanup EXIT
trap 'exit 130' INT
trap 'exit 143' TERM
[[ $(psql -v selector="$selector" -v marker="$marker" <<'SQL'
UPDATE identity.users SET display_name=:'marker',updated_at=now()
WHERE id=:'selector'::uuid RETURNING 'changed';
SQL
) == changed ]] || fail 'fixture user did not change'

before=$(snapshot)
bash scripts/backend.sh down --owner-file "$owner" >&2
owned || fail 'teardown owner mismatch'
SCREENROOM_BACKEND_BUILD=0 SCREENROOM_TELEMETRY_DISABLED=1 bash scripts/backend.sh up --owner-file "$owner" >&2
owned || fail 'teardown owner mismatch'
[[ $(snapshot) == "$before" ]] || fail 'database snapshot changed across restart'
[[ $(files) == "$bootstrap" ]] || fail 'installation identity changed across restart'
restore >/dev/null
[[ $(snapshot) == "$original" ]] || fail 'explicit restore did not match the original snapshot'
restored=yes
jq -nc --arg id "$instance" '{persistentRestart:true, operatorEditsPreserved:true, explicitRestore:true, instanceID:$id}'
