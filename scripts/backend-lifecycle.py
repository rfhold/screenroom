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
if 'SCREENROOM_BACKEND_PROFILE' in os.environ:
    raise SystemExit('backend profiles are retired; use the ordinary local installation')
PROJECT = os.environ.get('SCREENROOM_BACKEND_PROJECT', 'screenroom-local')
if not re.fullmatch(r'screenroom-[a-z0-9]+(?:-[a-z0-9]+)*', PROJECT):
    raise SystemExit('invalid backend project')
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
OPERATOR_COMPOSE = COMPOSE + ['--profile', 'index-operator']
os.environ['SCREENROOM_BACKEND_PORT'] = PORT
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


def installation_user():
    users = read_config(RUNTIME / 'users.json')
    if not isinstance(users, list) or not 1 <= len(users) <= 1000:
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
    selector_path = RUNTIME / 'local-user.json'
    descriptor_path = RUNTIME / 'descriptor.json'
    if selector_path.is_symlink() or descriptor_path.is_symlink():
        raise RuntimeError('runtime files must not be symlinks')
    selected = None
    if selector_path.exists():
        selector = read_config(selector_path)
        if not isinstance(selector, dict) or not isinstance(selector.get('userID'), str):
            raise RuntimeError('invalid installation user selector')
        selected = selector['userID']
    if not selected and descriptor_path.exists():
        descriptor = read_config(descriptor_path)
        if not isinstance(descriptor, dict) or not isinstance(descriptor.get('userID'), str):
            raise RuntimeError('invalid existing descriptor user selector')
        selected = descriptor['userID']
    if selected and selected not in ids:
        raise RuntimeError('persisted descriptor user is missing from users.json')
    selected = selected or str(uuid.UUID(users[0]['id']))
    create_json(selector_path, dict(userID=selected))
    if read_config(selector_path).get('userID') != selected:
        raise RuntimeError('installation user selector changed concurrently')
    return selected


def initialize_files():
    RUNTIME.mkdir(parents=True, exist_ok=True)
    if RUNTIME.is_symlink() or RUNTIME.stat().st_uid != os.getuid():
        raise RuntimeError('runtime must be an owned directory')
    os.chmod(RUNTIME, 0o700)
    for name in ('runtime.env', 'identity.json', 'users.json', 'descriptor.json', 'local-user.json'):
        path = RUNTIME / name
        if path.is_symlink():
            raise RuntimeError('runtime files must not be symlinks')
    if (RUNTIME / 'runtime.env').exists() and ((RUNTIME / 'runtime.env').stat().st_mode & 0o077 or (RUNTIME / 'runtime.env').stat().st_uid != os.getuid()):
        raise RuntimeError('runtime keys must remain owner-private')
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
            bootstrapRevision='installation-v1', publicOrigin=ORIGIN, clientNetwork=f'{PROJECT}_client'))
    persisted = read_config(identity)
    if not isinstance(persisted, dict) or persisted.get('publicOrigin') != ORIGIN or persisted.get('project') != PROJECT or persisted.get('clientNetwork') != f'{PROJECT}_client':
        raise RuntimeError('persistent instance origin mismatch')
    uuid.UUID(persisted['instanceID'])
    if not (RUNTIME / 'users.json').exists():
        if run(['docker', 'volume', 'ls', '-q', '--filter', f'label=com.docker.compose.project={PROJECT}'], capture=True).stdout.strip():
            raise RuntimeError('persistent volumes exist but user identity is missing; restore users.json')
        create_json(RUNTIME / 'users.json', [dict(id=str(uuid.uuid4()), display_name='Local user', links=[])])
    installation_user()


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
        args += ['-H', f'x-screenroom-user-id: {installation_user()}']
    args += ['-d', json.dumps(data or {}), f'127.0.0.1:{PORT}', method]
    return json.loads(run(args, capture=True).stdout)


def validate_embeddings():
    from urllib.parse import urlsplit
    keys = ('BASE_URL', 'MODEL', 'DIMENSIONS', 'REVISION', 'API_KEY')
    values = {key: os.environ.get('SCREENROOM_EMBEDDING_' + key) for key in keys}
    if not any(value is not None for value in values.values()):
        return
    url = urlsplit(values['BASE_URL'] or '')
    if (url.scheme not in ('http', 'https') or not url.hostname or url.username or url.password
            or url.query or url.fragment or values['DIMENSIONS'] != '1024'
            or any(not values[key] or len(values[key].encode()) > 200 for key in ('MODEL', 'REVISION'))):
        raise RuntimeError('invalid optional embedding configuration; provide URL, model, 1024 dimensions and revision')


def initialize_credentials():
    script = '''set -eu
d=/run/screenroom/credentials
test -d "$d" && test ! -L "$d"
if [ "$(stat -c '%u:%g:%a' "$d")" = '0:0:755' ] && [ -z "$(ls -A "$d")" ]; then
  chown 10001:10001 "$d"
  chmod 700 "$d"
fi
test "$(stat -c '%u:%g:%a' "$d")" = '10001:10001:700'
'''
    operator_compose('run', '--rm', '--no-deps', '--user', '0:0', '--entrypoint', '/bin/sh',
                     'index-operator', '-ec', script, capture=True)


def provision_operator_role():
    sql("""DO $$ BEGIN
IF NOT EXISTS (SELECT FROM pg_roles WHERE rolname='operator_runtime') THEN
 CREATE ROLE operator_runtime LOGIN INHERIT PASSWORD 'operator-local-only' IN ROLE screenroom_operator;
END IF;
END $$;""")


def worker_command(command, timeout=1800):
    name = f'{PROJECT}-operation-{uuid.uuid4().hex}'
    try:
        return bounded_run(COMPOSE + ['run', '--rm', '--no-deps', '--name', name,
                           'worker', 'screenroom-index-worker', *command],
                           cwd=ROOT, timeout=timeout, cap=1024 * 1024, group=True)
    except (subprocess.TimeoutExpired, RuntimeError):
        # The CLI's process group does not own the daemon's one-off container.
        bounded_run(['docker', 'stop', '--time', '15', name], cwd=ROOT, timeout=30, check=False)
        raise


def wait():
    seconds = int(os.environ.get('SCREENROOM_BACKEND_WAIT_SECONDS', '120'))
    if not 1 <= seconds <= 900:
        raise RuntimeError('wait must be between 1 and 900 seconds')
    spec = importlib.util.spec_from_file_location('contract', ROOT / 'scripts/backend-contract.py')
    contract = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(contract)
    deadline = time.monotonic() + seconds
    while time.monotonic() < deadline:
        try:
            info = rpc('screenroom.catalog.v1.CatalogService/GetCatalogInfo', user=False)
            current = rpc('screenroom.user.v1.UserService/GetCurrentUser')
            if (contract.web_rpc('screenroom.catalog.v1.CatalogService/GetCatalogInfo') != info
                    or contract.web_rpc('screenroom.user.v1.UserService/GetCurrentUser') != current):
                raise RuntimeError('native and browser readiness mismatch')
            with urllib.request.build_opener(urllib.request.ProxyHandler({})).open(ORIGIN + '/__screenroom_instance', timeout=5) as response:
                identity = json.load(response)
            expected = read_config(RUNTIME / 'identity.json')
            if identity != expected or current.get('user', {}).get('id') != installation_user():
                raise RuntimeError('public identity or RPC mismatch')
            descriptor = dict(expected, userID=installation_user(),
                clientUpstream='edge:50051', ready=True,
                checks=dict(native=True, grpcWeb=True, identity=True))
            temporary = RUNTIME / f'.descriptor-{uuid.uuid4()}.json'
            temporary.write_text(json.dumps(descriptor, indent=2))
            temporary.replace(RUNTIME / 'descriptor.json')
            print(RUNTIME / 'descriptor.json')
            return
        except (subprocess.SubprocessError, ValueError, OSError, RuntimeError, AssertionError):
            time.sleep(min(2, max(0, deadline - time.monotonic())))
    raise RuntimeError('readiness timed out; inspect private service diagnostics')


def operator_settings():
    port = os.environ.get('SCREENROOM_OPERATOR_PORT', '50072')
    reserved = {4173, 4176, 5173, 50051, 50052, 50073, int(PORT)}
    if not port.isascii() or not port.isdigit() or str(int(port)) != port or not 1024 <= int(port) <= 65535 or int(port) in reserved:
        raise RuntimeError('invalid or conflicting operator port')
    origin = f'http://127.0.0.1:{port}'
    if os.environ.get('SCREENROOM_OPERATOR_ORIGIN', origin) != origin:
        raise RuntimeError(f'SCREENROOM_OPERATOR_ORIGIN must equal {origin}')
    os.environ.update(SCREENROOM_OPERATOR_PORT=port, SCREENROOM_OPERATOR_ORIGIN=origin,
                      SCREENROOM_OPERATOR_IMAGE='screenroom-backend:local')
    return origin


def operator_instance():
    if not RUNTIME.is_dir() or RUNTIME.is_symlink() or RUNTIME.stat().st_uid != os.getuid():
        raise RuntimeError('operator requires an existing owned runtime')
    for name in ('runtime.env', 'identity.json', 'descriptor.json', 'lifecycle.lock'):
        path = RUNTIME / name
        if path.is_symlink() or not path.is_file() or path.stat().st_uid != os.getuid():
            raise RuntimeError(f'operator requires existing owned {name}')
    if (RUNTIME / 'runtime.env').stat().st_mode & 0o077:
        raise RuntimeError('runtime keys must remain owner-private')
    identity = read_config(RUNTIME / 'identity.json')
    descriptor = read_config(RUNTIME / 'descriptor.json')
    expected = dict(project=PROJECT, publicOrigin=ORIGIN, clientNetwork=f'{PROJECT}_client')
    if not isinstance(identity, dict) or any(identity.get(k) != v for k, v in expected.items()):
        raise RuntimeError('operator instance ownership mismatch')
    uuid.UUID(identity['instanceID'])
    if not isinstance(descriptor, dict) or descriptor.get('ready') is not True or any(descriptor.get(k) != v for k, v in identity.items()):
        raise RuntimeError('operator requires matching ready instance descriptor')
    return identity


def operator_compose(*args, capture=False):
    return run(OPERATOR_COMPOSE + list(args), capture=capture)


def operator_database():
    container = operator_compose('ps', '-q', 'postgres', capture=True).stdout.strip()
    if not container or '\n' in container:
        raise RuntimeError('operator requires existing healthy postgres')
    raw = run(['docker', 'inspect', '--format', '{{json .State}}|{{json .Config.Labels}}', container], capture=True).stdout
    state, labels = (json.loads(part) for part in raw.strip().split('|', 1))
    if state.get('Health', {}).get('Status') != 'healthy' or not state.get('Running') or labels.get('com.docker.compose.project') != PROJECT or labels.get('com.docker.compose.service') != 'postgres' or labels.get('com.docker.compose.project.working_dir') != str(ROOT):
        raise RuntimeError('operator postgres health or ownership mismatch')
    role = operator_compose('exec', '-T', 'postgres', 'psql', '-XAt', '-v', 'ON_ERROR_STOP=1',
        '-U', 'screenroom', '-d', 'screenroom', '-c',
        "SELECT count(*) FROM pg_roles WHERE rolname='operator_runtime' AND rolcanlogin AND NOT rolsuper AND NOT rolcreatedb AND NOT rolcreaterole AND pg_has_role(oid,'screenroom_operator','member')", capture=True).stdout.strip()
    if role != '1':
        raise RuntimeError('operator requires existing restricted operator runtime database role')


def operator_container(origin):
    container = operator_compose('ps', '-q', 'index-operator', capture=True).stdout.strip()
    if not container or '\n' in container:
        raise RuntimeError('operator requires one running container owned by this project')
    raw = run(['docker', 'inspect', '--format',
               '{{json .State}}|{{json .Config.Labels}}|{{json .NetworkSettings.Ports}}', container], capture=True).stdout
    state, labels, ports = (json.loads(part) for part in raw.strip().split('|', 2))
    if not state.get('Running') or labels.get('com.docker.compose.project') != PROJECT or labels.get('com.docker.compose.service') != 'index-operator' or labels.get('com.docker.compose.project.working_dir') != str(ROOT):
        raise RuntimeError('operator container ownership or running state mismatch')
    expected = [{'HostIp': '127.0.0.1', 'HostPort': origin.rsplit(':', 1)[1]}]
    if not isinstance(ports, dict) or ports.get('50072/tcp') != expected:
        raise RuntimeError('operator container published port does not match requested loopback origin')


def operator_health(origin):
    request = urllib.request.Request(origin + '/api/health', headers={'Host': origin.removeprefix('http://')})
    with urllib.request.build_opener(urllib.request.ProxyHandler({})).open(request, timeout=3) as response:
        if response.geturl() != request.full_url or json.load(response).get('ready') is not True:
            raise RuntimeError('operator is not ready')


def operator_public_identity(identity):
    with urllib.request.build_opener(urllib.request.ProxyHandler({})).open(ORIGIN + '/__screenroom_instance', timeout=3) as response:
        if response.geturl() != ORIGIN + '/__screenroom_instance' or json.load(response) != identity:
            raise RuntimeError('operator public instance identity mismatch')


def operator_command(command, owner_file=None):
    origin = operator_settings()
    identity = operator_instance()
    if owner_file and (not owner_file.is_file() or owner_file.read_text() != PROJECT):
        raise RuntimeError('operator owner mismatch')
    with (RUNTIME / 'lifecycle.lock').open('r+') as lock:
        fcntl.flock(lock, (fcntl.LOCK_SH if command == 'operator-status' else fcntl.LOCK_EX) | fcntl.LOCK_NB)
        if command == 'operator-down':
            operator_compose('stop', '--timeout', '15', 'index-operator')
            return
        if command == 'operator-up':
            operator_public_identity(identity)
            operator_database()
            operator_compose('up', '-d', '--no-deps', '--no-build', '--pull', 'never', 'index-operator')
            deadline = time.monotonic() + 60
            while True:
                try:
                    operator_container(origin)
                    operator_health(origin)
                    break
                except (OSError, ValueError, RuntimeError):
                    if time.monotonic() >= deadline:
                        raise RuntimeError('operator readiness timed out; inspect private operator logs')
                    time.sleep(1)
        else:
            operator_public_identity(identity)
            operator_container(origin)
            operator_health(origin)
        print(json.dumps(dict(project=PROJECT, instanceID=identity['instanceID'], operatorOrigin=origin, ready=True)))


def main():
    parser = argparse.ArgumentParser()
    parser.add_argument('command', choices=['initialize', 'up', 'wait', 'status', 'drain', 'activate-model', 'down',
                                          'operator-up', 'operator-down', 'operator-status'])
    parser.add_argument('--owner-file', type=Path)
    args = parser.parse_args()
    if args.command.startswith('operator-'):
        operator_command(args.command, args.owner_file)
        return
    if args.command in ('initialize', 'up', 'drain', 'activate-model'):
        validate_embeddings()
    if args.command == 'status':
        descriptor = json.loads((RUNTIME / 'descriptor.json').read_text())
        with urllib.request.build_opener(urllib.request.ProxyHandler({})).open(ORIGIN + '/__screenroom_instance', timeout=3) as response:
            identity = json.load(response)
        expected = json.loads((RUNTIME / 'identity.json').read_text())
        if identity != expected or any(descriptor.get(key) != value for key, value in expected.items()) or descriptor.get('ready') is not True or descriptor.get('userID') != installation_user():
            raise RuntimeError('instance identity or user selector mismatch')
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
            initialize_credentials()
            compose('run', '--rm', '--no-deps', 'user', 'screenroom-user-init', '--create-missing', '/runtime/users.json')
            provision_operator_role()
            compose('up', '-d', 'user', 'catalog', 'worker', 'edge')
            wait()
        elif args.command == 'initialize':
            initialize_files()
        elif args.command in ('drain', 'activate-model'):
            worker_command([args.command])
        elif args.command == 'wait':
            wait()
        else:
            if args.owner_file and (not args.owner_file.is_file() or args.owner_file.read_text() != PROJECT):
                raise RuntimeError('teardown owner mismatch')
            compose('down', '--timeout', '15')
            (RUNTIME / 'descriptor.json').unlink(missing_ok=True)


if __name__ == '__main__':
    try:
        main()
    except (RuntimeError, subprocess.CalledProcessError, OSError, ValueError) as error:
        print('backend: operation failed; inspect private diagnostics' if isinstance(error, subprocess.CalledProcessError) else f'backend: {error}', file=sys.stderr)
        sys.exit(1)
