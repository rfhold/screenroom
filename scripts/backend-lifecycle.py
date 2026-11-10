#!/usr/bin/env python3
"""Persistent local Compose lifecycle. No volume deletion or implicit external attachment."""
import argparse
import fcntl
import importlib.util
import json
import os
from pathlib import Path
import re
import secrets
import subprocess
import sys
import tempfile
import time
import urllib.request
import uuid

ROOT = Path(__file__).resolve().parent.parent
PROJECT = os.environ.get('SCREENROOM_BACKEND_PROJECT', 'screenroom-e2e')
if not re.fullmatch(r'screenroom-e2e(?:-[a-z0-9-]+)?', PROJECT):
    raise SystemExit('project must be screenroom-e2e or screenroom-e2e-<suffix>')
RUNTIME = ROOT / 'build/backend-runtime' / PROJECT
PORT = os.environ.get('SCREENROOM_BACKEND_PORT', '50051')
if not PORT.isdigit() or not 1024 <= int(PORT) <= 65535:
    raise SystemExit('invalid explicit backend port')
ORIGIN = f'http://127.0.0.1:{PORT}'
for key in ('SCREENROOM_API_ENDPOINT', 'SCREENROOM_E2E_API_ENDPOINT'):
    if os.environ.get(key, ORIGIN) != ORIGIN:
        raise SystemExit(f'{key} must equal {ORIGIN}')
os.environ.update(SCREENROOM_BACKEND_ENV=str(RUNTIME / 'runtime.env'),
                  SCREENROOM_BACKEND_RUNTIME=str(RUNTIME), SCREENROOM_API_ENDPOINT=ORIGIN)
COMPOSE = ['docker', 'compose', '--env-file', str(RUNTIME / 'runtime.env'), '-p', PROJECT, '-f', str(ROOT / 'compose.yaml')]
sys.dont_write_bytecode = True
_spec = importlib.util.spec_from_file_location('backend_process', ROOT / 'scripts/backend-process.py')
_process = importlib.util.module_from_spec(_spec)
_spec.loader.exec_module(_process)
bounded_run = _process.bounded_run


def run(args, capture=False, check=True):
    if capture:
        return bounded_run(args, cwd=ROOT, check=check, timeout=45)
    return subprocess.run(args, cwd=ROOT, check=check, text=True,
                          stdout=subprocess.PIPE if capture else sys.stderr,
                          stderr=subprocess.PIPE if capture else None)


def compose(*args, capture=False, check=True):
    return run(COMPOSE + list(args), capture, check)


def garage(*args, check=True):
    result = compose('exec', '-T', 'garage', '/garage', *args, capture=True, check=False)
    if check and result.returncode:
        raise RuntimeError(f'Garage {args[0]} operation failed; inspect private Garage logs')
    return result


def sql(query):
    return compose('exec', '-T', 'postgres', 'psql', '-XAt', '-v', 'ON_ERROR_STOP=1',
                   '-U', 'screenroom', '-d', 'screenroom', '-c', query, capture=True).stdout.strip()


def create_json(path, value):
    if path.exists():
        return
    fd, temporary = tempfile.mkstemp(prefix='.bootstrap-', dir=path.parent)
    try:
        with os.fdopen(fd, 'w') as file:
            json.dump(value, file)
            file.flush()
            os.fsync(file.fileno())
        os.chmod(temporary, 0o644)
        try:
            os.link(temporary, path)
        except FileExistsError:
            pass
    finally:
        os.unlink(temporary)


def read_config(path):
    with path.open('rb') as file:
        data = file.read(1024 * 1024 + 1)
    if len(data) > 1024 * 1024:
        raise RuntimeError(f'{path.name} exceeds 1 MiB')
    try:
        return json.loads(data)
    except (ValueError, UnicodeError) as error:
        raise RuntimeError(f'invalid {path.name} JSON') from error


def validate_bootstrap():
    installation = read_config(RUNTIME / 'installation.json')
    sources = installation.get('sources') if isinstance(installation, dict) else None
    if not isinstance(sources, list) or len(sources) > 32:
        raise RuntimeError('invalid existing installation.json sources')
    keys = set()
    for source in sources:
        if not isinstance(source, dict) or source.get('provider') not in ('jellyfin', 'tmdb'):
            raise RuntimeError('invalid existing installation.json provider')
        key = source.get('key')
        if not isinstance(key, str) or not key or key in keys:
            raise RuntimeError('invalid existing installation.json source key')
        keys.add(key)
        configuration = source.get('configuration')
        if not isinstance(configuration, dict) or configuration.get('adapter') != 'mock':
            raise RuntimeError('installation requires offline mock adapters')
    users = read_config(RUNTIME / 'users.json')
    if not isinstance(users, list) or len(users) > 1000:
        raise RuntimeError('invalid existing users.json')
    ids = set()
    for user in users:
        if not isinstance(user, dict) or not isinstance(user.get('display_name'), str):
            raise RuntimeError('invalid existing users.json user')
        try:
            identity = str(uuid.UUID(user.get('id', '')))
        except (ValueError, TypeError, AttributeError) as error:
            raise RuntimeError('invalid existing users.json identity') from error
        if identity in ids:
            raise RuntimeError('duplicate existing users.json identity')
        ids.add(identity)


def initialize_files():
    RUNTIME.mkdir(parents=True, exist_ok=True)
    env = RUNTIME / 'runtime.env'
    if not env.exists():
        existing = run(['docker', 'volume', 'ls', '-q', '--filter', f'label=com.docker.compose.project={PROJECT}'], capture=True).stdout.strip()
        if existing:
            raise RuntimeError('persistent volumes exist but runtime keys are missing; restore runtime.env, never rotate implicitly')
        fd = os.open(env, os.O_WRONLY | os.O_CREAT | os.O_EXCL, 0o600)
        with os.fdopen(fd, 'w') as file:
            file.write(f'GARAGE_RPC_SECRET={secrets.token_hex(32)}\n')
            for role in ('CATALOG', 'WORKER'):
                file.write(f'{role}_S3_KEY=GK{secrets.token_hex(12)}\n')
                file.write(f'{role}_S3_SECRET={secrets.token_hex(32)}\n')
    identity = RUNTIME / 'identity.json'
    if not identity.exists():
        if run(['docker', 'volume', 'ls', '-q', '--filter', f'label=com.docker.compose.project={PROJECT}'], capture=True).stdout.strip():
            raise RuntimeError('persistent volumes exist but instance identity is missing; restore identity.json')
        create_json(identity, dict(instanceID=str(uuid.uuid4()), project=PROJECT,
            seedRevision='mock-adapters-v1', publicOrigin=ORIGIN, clientNetwork=f'{PROJECT}_client'))
    persisted = read_config(identity)
    if not isinstance(persisted, dict) or persisted.get('publicOrigin') != ORIGIN or persisted.get('project') != PROJECT or persisted.get('clientNetwork') != f'{PROJECT}_client' or persisted.get('seedRevision') != 'mock-adapters-v1':
        raise RuntimeError('persistent instance origin mismatch')
    uuid.UUID(persisted['instanceID'])
    notices = json.loads((ROOT / 'fixtures/catalog/notices.json').read_text())
    policy = dict(enabled=True, max_items=1000, max_requests=100,
                  incremental_seconds=86400, reconcile_seconds=604800,
                  enrich_metadata=True, enrich_people=True, enrich_artwork=True)
    feeds = []
    for feed in ('MoviePopular', 'MovieUpcoming', 'MovieNowPlaying', 'MovieTrending',
                 'TvPopular', 'TvOnTheAir', 'TvTrending'):
        trending = feed.endswith('Trending')
        feeds.append(dict(feed_key=f'{feed}:en-US:{"Day" if trending else "US"}:1', feed=feed,
                          max_pages=1, page_size=25, locale='en-US',
                          region='US' if feed in ('MoviePopular', 'MovieUpcoming', 'MovieNowPlaying') else None,
                          trending_window='Day' if trending else None))
    sources = []
    for provider in ('jellyfin', 'tmdb'):
        selection = dict(library_ids=['movies', 'tv'], page_size=25) if provider == 'jellyfin' else dict(feeds=feeds)
        sources.append(dict(key=f'mock-{provider}', provider=provider,
            configuration=dict(adapter='mock', notices=notices if provider == 'tmdb' else [], branding_artwork_ids=['tmdb']),
            capabilities={}, libraries=[dict(id=id, external_id=id, title=title)
                for id, title in (('movies', 'Movies'), ('tv', 'TV'))] if provider == 'jellyfin' else [],
            policy=dict(policy, selection_json=selection)))
    create_json(RUNTIME / 'installation.json', dict(sources=sources))
    create_json(RUNTIME / 'users.json', [dict(id=f'00000000-0000-0000-0000-00000000000{i}',
        display_name=name, links=[dict(source_id='mock-jellyfin', jellyfin_user_id=f'mock-jellyfin-{name.lower()}')])
        for i, name in ((1, 'Alice'), (2, 'Bob'))])
    validate_bootstrap()


def bootstrap_garage():
    deadline = time.monotonic() + 90
    while time.monotonic() < deadline:
        status = garage('status', check=False)
        if status.returncode == 0:
            break
        time.sleep(1)
    else:
        raise RuntimeError('Garage did not start')
    layout = garage('layout', 'show').stdout
    if 'Current cluster layout version: 0' in layout:
        node = re.search(r'^([0-9a-f]{16,})\s', status.stdout, re.M)
        if not node:
            raise RuntimeError('cannot find Garage node identity')
        garage('layout', 'assign', '-z', 'local', '-c', '1G', node[1])
        garage('layout', 'apply', '--version', '1')
    values = dict(line.split('=', 1) for line in (RUNTIME / 'runtime.env').read_text().splitlines())
    if garage('bucket', 'info', 'screenroom-artwork', check=False).returncode:
        garage('bucket', 'create', 'screenroom-artwork')
    for role in ('CATALOG', 'WORKER'):
        key = values[f'{role}_S3_KEY']
        if garage('key', 'info', key, check=False).returncode:
            garage('key', 'import', '--yes', '-n', f'screenroom-{role.lower()}', key, values[f'{role}_S3_SECRET'])
        garage('bucket', 'allow', 'screenroom-artwork', '--key', key, '--read',
               *(['--write'] if role == 'WORKER' else []))


def rpc(method, data=None, user=True):
    args = ['grpcurl', '-plaintext', '-max-time', '15', '-import-path', 'proto',
            '-proto', 'screenroom/catalog/v1/catalog.proto', '-proto', 'screenroom/user/v1/user.proto']
    if user:
        args += ['-H', 'x-screenroom-user-id: 00000000-0000-0000-0000-000000000001']
    args += ['-d', json.dumps(data or {}), f'127.0.0.1:{PORT}', method]
    return json.loads(run(args, capture=True).stdout)


def wait(activate=False):
    deadline = time.monotonic() + int(os.environ.get('SCREENROOM_BACKEND_WAIT_SECONDS', '900'))
    last = 'not checked'
    while time.monotonic() < deadline:
        try:
            pending = sql("SELECT count(*) FROM ingest.jobs WHERE state NOT IN ('complete','cancelled')")
            docs = int(sql('SELECT count(*) FROM search.documents'))
            if pending != '0' or docs < 10:
                raise RuntimeError(f'jobs={pending}; documents={docs}')
            if activate and sql("SELECT count(*) FROM search.embedding_models WHERE state='active'") == '0':
                compose('run', '--rm', '--no-deps', 'worker', 'screenroom-index-worker', 'activate-model', capture=True)
            if sql("SELECT count(*) FROM search.embedding_models WHERE state='active' AND dimensions=1024") != '1':
                raise RuntimeError('no active 1024-dimensional model')
            missing = sql("SELECT count(*) FROM search.documents d JOIN catalog.source_items si ON si.id=d.source_item_id JOIN catalog.sources s ON s.id=si.source_id CROSS JOIN search.embedding_models m WHERE si.state='enabled' AND s.state='enabled' AND m.state='active' AND (d.document_version<>m.document_version OR NOT EXISTS(SELECT FROM search.embeddings e WHERE e.document_id=d.id AND e.model_id=m.id AND e.input_hash=d.content_hash))")
            if missing != '0':
                raise RuntimeError(f'active model missing {missing} current document embeddings')
            home = rpc('screenroom.catalog.v1.BrowseService/GetHome')
            user = rpc('screenroom.user.v1.UserService/GetCurrentUser')
            with urllib.request.urlopen(ORIGIN + '/__screenroom_instance', timeout=5) as response:
                identity = json.load(response)
            expected = json.loads((RUNTIME / 'identity.json').read_text())
            if identity != expected or not home or not user:
                raise RuntimeError('public instance or RPC mismatch')
            contract = bounded_run([sys.executable, 'scripts/backend-contract.py'], cwd=ROOT,
                timeout=min(120, max(0, deadline - time.monotonic())), cap=1024 * 1024, group=True)
            evidence = json.loads(contract.stdout)
            model = json.loads(sql("SELECT row_to_json(m) FROM (SELECT name,revision,dimensions,document_version FROM search.embedding_models WHERE state='active') m"))
            descriptor = RUNTIME / f'.descriptor-{uuid.uuid4()}.json'
            descriptor.write_text(json.dumps(dict(expected,
                userID='00000000-0000-0000-0000-000000000001', documents=docs,
                clientUpstream='edge:50051', embeddingModel=model,
                embeddingRevisionKind='operator-declared', checks=evidence, ready=True), indent=2))
            descriptor.replace(RUNTIME / 'descriptor.json')
            print(RUNTIME / 'descriptor.json')
            return
        except (subprocess.SubprocessError, ValueError, OSError, RuntimeError) as error:
            last = str(error)
            if isinstance(error, subprocess.CalledProcessError) and error.stderr:
                last += ': ' + error.stderr[-4000:]
            time.sleep(2)
    raise RuntimeError(f'semantic readiness timed out: {last}; inspect docker compose -p {PROJECT} logs')


def main():
    parser = argparse.ArgumentParser()
    parser.add_argument('command', choices=['up', 'wait', 'status', 'down'])
    parser.add_argument('--owner-file', type=Path)
    args = parser.parse_args()
    if args.command == 'status':
        descriptor = json.loads((RUNTIME / 'descriptor.json').read_text())
        with urllib.request.urlopen(ORIGIN + '/__screenroom_instance', timeout=3) as response:
            identity = json.load(response)
        expected = json.loads((RUNTIME / 'identity.json').read_text())
        if identity != expected or any(descriptor[key] != value for key, value in expected.items()):
            raise RuntimeError('live instance identity mismatch')
        print(json.dumps(descriptor, indent=2))
        return
    RUNTIME.mkdir(parents=True, exist_ok=True)
    with (RUNTIME / 'lifecycle.lock').open('a') as lock:
        mode = fcntl.LOCK_SH if args.command == 'wait' else fcntl.LOCK_EX
        fcntl.flock(lock, mode | fcntl.LOCK_NB)
        if args.command == 'up':
            if args.owner_file:
                running = run(['docker', 'ps', '-q', '--filter', f'label=com.docker.compose.project={PROJECT}'], capture=True).stdout.strip()
                if running:
                    raise RuntimeError('caller-owned project containers already running; use explicit attach mode')
                args.owner_file.write_text(PROJECT)
            initialize_files()
            compose('up', '-d', '--wait', 'postgres', 'garage')
            compose('exec', '-T', 'postgres', 'psql', '-v', 'ON_ERROR_STOP=1', '-U', 'screenroom', '-d', 'screenroom', '-f', '/docker-entrypoint-initdb.d/10-runtime.sql')
            bootstrap_garage()
            build = os.environ.get('SCREENROOM_BACKEND_BUILD', '1')
            if build not in ('0', '1'):
                raise RuntimeError('SCREENROOM_BACKEND_BUILD must be 0 or 1')
            if build == '1':
                compose('build', 'catalog')
            compose('run', '--rm', '--no-deps', 'catalog', 'screenroom-catalog-init', '/runtime/installation.json')
            compose('run', '--rm', '--no-deps', 'user', 'screenroom-user-init', '--create-missing', '/runtime/users.json')
            compose('up', '-d', 'user', 'catalog', 'worker', 'edge')
            wait(activate=True)
        elif args.command == 'wait':
            wait()
        else:
            compose('down', '--timeout', '15')
            (RUNTIME / 'descriptor.json').unlink(missing_ok=True)


if __name__ == '__main__':
    try:
        main()
    except (RuntimeError, subprocess.CalledProcessError, OSError) as error:
        print(f'backend: {error}', file=sys.stderr)
        sys.exit(1)
