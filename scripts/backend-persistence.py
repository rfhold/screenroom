#!/usr/bin/env python3
"""Explicit local acceptance: restart the task-owned stack, preserving all volumes."""
import importlib.util
import json
import os
import sys
import uuid
from pathlib import Path

sys.dont_write_bytecode = True
spec = importlib.util.spec_from_file_location('backend', Path(__file__).with_name('backend-lifecycle.py'))
backend = importlib.util.module_from_spec(spec)
spec.loader.exec_module(backend)


def snapshot():
    tables = ('identity.users', 'identity.jellyfin_user_links', 'catalog.sources',
              'catalog.items', 'catalog.source_items', 'ingest.policies', 'media.objects',
              'media.artwork', 'media.variants', 'search.documents', 'search.embedding_models',
              'search.embeddings')
    return {table: backend.sql(f"SELECT md5(coalesce(string_agg(r::text, '' ORDER BY r::text),'')) FROM (SELECT row_to_json(t) AS r FROM {table} t) rows") for table in tables}


def literal(value):
    return "'" + json.dumps(value).replace("'", "''") + "'"


def main():
    backend.run(['bash', 'scripts/backend.sh', 'wait'])
    selector = "('00000000-0000-0000-0000-000000000001','00000000-0000-0000-0000-000000000002')"
    users = json.loads(backend.sql(f"SELECT json_agg(t) FROM identity.users t WHERE id IN {selector}"))
    links = json.loads(backend.sql(f"SELECT coalesce(json_agg(t),'[]') FROM identity.jellyfin_user_links t WHERE user_id IN {selector}"))
    assert len(users) == 2
    recovery = backend.RUNTIME / f'operator-restore-{uuid.uuid4()}.json'
    recovery.write_text(json.dumps(dict(users=users, links=links)))
    os.chmod(recovery, 0o600)
    original = snapshot()
    bootstrap = {name: (backend.RUNTIME / name).read_bytes() for name in ('installation.json', 'users.json')}
    identity = (backend.RUNTIME / 'identity.json').read_text()
    try:
        backend.sql("BEGIN; UPDATE identity.users SET display_name='Alice operator edit',updated_at=now() WHERE id='00000000-0000-0000-0000-000000000001'; UPDATE identity.users SET state='disabled',updated_at=now() WHERE id='00000000-0000-0000-0000-000000000002'; UPDATE identity.jellyfin_user_links SET jellyfin_user_id='mock-jellyfin-bob',updated_at=now() WHERE user_id='00000000-0000-0000-0000-000000000001' AND source_id='mock-jellyfin'; DELETE FROM identity.jellyfin_user_links WHERE user_id='00000000-0000-0000-0000-000000000002'; COMMIT;")
        assert backend.sql("SELECT count(*) FROM identity.users WHERE (id='00000000-0000-0000-0000-000000000001' AND display_name='Alice operator edit') OR (id='00000000-0000-0000-0000-000000000002' AND state='disabled')") == '2'
        assert backend.sql("SELECT count(*) FROM identity.jellyfin_user_links WHERE user_id='00000000-0000-0000-0000-000000000001' AND source_id='mock-jellyfin' AND jellyfin_user_id='mock-jellyfin-bob'") == '1'
        assert backend.sql("SELECT count(*) FROM identity.jellyfin_user_links WHERE user_id='00000000-0000-0000-0000-000000000002'") == '0'
        before = snapshot()
        backend.compose('run', '--rm', '--no-deps', 'user', 'screenroom-user-init', '--create-missing', '/runtime/users.json')
        assert before == snapshot(), 'repeated bootstrap overwrote operator state'
        backend.run(['bash', 'scripts/backend.sh', 'down'])
        os.environ['SCREENROOM_BACKEND_BUILD'] = '0'
        backend.run(['bash', 'scripts/backend.sh', 'up'])
        backend.compose('run', '--rm', '--no-deps', 'user', 'screenroom-user-init', '--create-missing', '/runtime/users.json')
        after = snapshot()
        assert before == after, {key: (before[key], after[key]) for key in before if before[key] != after[key]}
        assert identity == (backend.RUNTIME / 'identity.json').read_text()
        assert bootstrap == {name: (backend.RUNTIME / name).read_bytes() for name in bootstrap}
    finally:
        backend.compose('up', '-d', '--wait', 'postgres')
        backend.sql(f"BEGIN; INSERT INTO identity.users SELECT * FROM json_populate_recordset(NULL::identity.users,{literal(users)}) ON CONFLICT(id) DO UPDATE SET display_name=EXCLUDED.display_name,state=EXCLUDED.state,created_at=EXCLUDED.created_at,updated_at=EXCLUDED.updated_at; DELETE FROM identity.jellyfin_user_links WHERE user_id IN {selector}; INSERT INTO identity.jellyfin_user_links SELECT * FROM json_populate_recordset(NULL::identity.jellyfin_user_links,{literal(links)}); COMMIT;")
        assert original == snapshot(), f'explicit operator restore differs; preserve {recovery}'
        recovery.unlink()
    backend.run(['bash', 'scripts/backend.sh', 'wait'])
    print(json.dumps(dict(persistentRestart=True, operatorEditsPreserved=True, explicitRestore=True,
                         preservedTables=list(original), instanceID=json.loads(identity)['instanceID'])))


if __name__ == '__main__':
    main()
