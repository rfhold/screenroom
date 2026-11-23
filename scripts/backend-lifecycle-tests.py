#!/usr/bin/env python3
import importlib.util
import json
import os
from pathlib import Path
import struct
import subprocess
import sys
import tempfile
from types import SimpleNamespace
import unittest
from unittest.mock import MagicMock, patch
import uuid
import yaml

sys.dont_write_bytecode = True
spec = importlib.util.spec_from_file_location('backend', Path(__file__).with_name('backend-lifecycle.py'))
backend = importlib.util.module_from_spec(spec)
spec.loader.exec_module(backend)
spec = importlib.util.spec_from_file_location('contract', Path(__file__).with_name('backend-contract.py'))
contract = importlib.util.module_from_spec(spec)
spec.loader.exec_module(contract)


class LifecycleTests(unittest.TestCase):
    def setUp(self):
        self.directory = tempfile.TemporaryDirectory(dir=backend.ROOT / 'build')
        self.runtime = Path(self.directory.name)
        self.runtime_patch = patch.object(backend, 'RUNTIME', self.runtime)
        self.runtime_patch.start()
        self.docker_patch = patch.object(backend, 'run', return_value=SimpleNamespace(stdout=''))
        self.docker_patch.start()

    def tearDown(self):
        self.docker_patch.stop()
        self.runtime_patch.stop()
        self.directory.cleanup()

    def prepare(self):
        backend.initialize_files()
        identity = backend.read_config(self.runtime / 'identity.json')
        (self.runtime / 'descriptor.json').write_text(json.dumps(dict(identity, userID=backend.installation_user(), ready=True)))
        (self.runtime / 'lifecycle.lock').touch()
        return identity

    def test_fresh_installation_has_one_random_non_demo_user_and_no_sources(self):
        backend.initialize_files()
        users = backend.read_config(self.runtime / 'users.json')
        self.assertEqual(len(users), 1)
        self.assertEqual(uuid.UUID(users[0]['id']).version, 4)
        self.assertEqual(users[0]['display_name'], 'Local user')
        self.assertEqual(users[0]['links'], [])
        self.assertFalse((self.runtime / 'installation.json').exists())
        self.assertEqual(backend.installation_user(), users[0]['id'])
        self.assertEqual(self.runtime.stat().st_mode & 0o777, 0o700)
        self.assertEqual((self.runtime / 'runtime.env').stat().st_mode & 0o777, 0o600)

    def test_reinitialization_preserves_all_operator_state_and_old_identity_format(self):
        self.prepare()
        identity = backend.read_config(self.runtime / 'identity.json')
        identity.pop('bootstrapRevision')
        identity['seedRevision'] = 'previous-format'
        (self.runtime / 'identity.json').write_text(json.dumps(identity))
        users = backend.read_config(self.runtime / 'users.json')
        users[0].update(display_name='Operator name', links=[dict(source_id='existing', jellyfin_user_id='profile')])
        (self.runtime / 'users.json').write_text(json.dumps(users, indent=3))
        (self.runtime / 'installation.json').write_text('obsolete configuration is not consumed')
        before = {p: p.read_bytes() for p in self.runtime.iterdir() if p.is_file()}
        backend.initialize_files()
        self.assertEqual(before, {p: p.read_bytes() for p in before})

    def test_missing_identity_or_keys_with_existing_volumes_is_not_recreated(self):
        for missing in ('runtime.env', 'identity.json', 'users.json'):
            backend.initialize_files()
            (self.runtime / missing).unlink()
            with patch.object(backend, 'run', return_value=SimpleNamespace(stdout='owned-volume')):
                with self.assertRaisesRegex(RuntimeError, 'restore ' + missing):
                    backend.initialize_files()
            self.assertFalse((self.runtime / missing).exists())

    def test_invalid_users_and_wrong_origin_are_rejected_without_overwrite(self):
        backend.initialize_files()
        path = self.runtime / 'users.json'
        for value in ({}, [], [dict(id='invalid', display_name='Name')]):
            path.write_text(json.dumps(value))
            with self.assertRaises(RuntimeError):
                backend.initialize_files()
            self.assertEqual(json.loads(path.read_text()), value)
        identity = backend.read_config(self.runtime / 'identity.json')
        identity['publicOrigin'] = 'http://127.0.0.1:49999'
        (self.runtime / 'identity.json').write_text(json.dumps(identity))
        with self.assertRaisesRegex(RuntimeError, 'origin mismatch'):
            backend.initialize_files()

    def test_runtime_symlinks_and_public_key_permissions_are_rejected(self):
        backend.initialize_files()
        (self.runtime / 'runtime.env').chmod(0o644)
        with self.assertRaisesRegex(RuntimeError, 'owner-private'):
            backend.initialize_files()
        (self.runtime / 'runtime.env').unlink()
        (self.runtime / 'runtime.env').symlink_to(self.runtime / 'identity.json')
        with self.assertRaisesRegex(RuntimeError, 'symlinks'):
            backend.initialize_files()

    def test_atomic_create_does_not_overwrite_a_winner(self):
        from concurrent.futures import ThreadPoolExecutor
        path = self.runtime / 'race.json'
        with ThreadPoolExecutor(max_workers=8) as executor:
            list(executor.map(lambda value: backend.create_json(path, {'winner': value}), range(20)))
        winner = path.read_bytes()
        backend.create_json(path, {'winner': -1})
        self.assertEqual(path.read_bytes(), winner)

    def test_selector_uses_persisted_descriptor_and_rpc_header(self):
        backend.initialize_files()
        users = backend.read_config(self.runtime / 'users.json')
        selected = str(uuid.uuid4())
        users.append(dict(id=selected, display_name='Second local user'))
        (self.runtime / 'users.json').write_text(json.dumps(users))
        (self.runtime / 'local-user.json').unlink()
        (self.runtime / 'descriptor.json').write_text(json.dumps(dict(userID=selected)))
        with patch.object(backend, 'run', return_value=SimpleNamespace(stdout='{}')) as run:
            backend.rpc('example/Read')
        self.assertIn(f'x-screenroom-user-id: {selected}', run.call_args.args[0])
        (self.runtime / 'descriptor.json').unlink()
        self.assertEqual(backend.installation_user(), selected)
        (self.runtime / 'local-user.json').write_text(json.dumps(dict(userID=str(uuid.uuid4()))))
        with self.assertRaisesRegex(RuntimeError, 'missing from users'):
            backend.installation_user()

    def test_optional_embeddings_validate_without_endpoint_approval_or_network(self):
        with patch.dict(os.environ, {}, clear=True):
            backend.validate_embeddings()
        valid = dict(SCREENROOM_EMBEDDING_BASE_URL='https://example.invalid/v1', SCREENROOM_EMBEDDING_MODEL='model',
                     SCREENROOM_EMBEDDING_DIMENSIONS='1024', SCREENROOM_EMBEDDING_REVISION='revision')
        with patch.dict(os.environ, valid, clear=True):
            backend.validate_embeddings()
        for changed in (dict(SCREENROOM_EMBEDDING_DIMENSIONS='3'), dict(SCREENROOM_EMBEDDING_MODEL=''),
                        dict(SCREENROOM_EMBEDDING_BASE_URL='https://user:secret@example.invalid'),
                        dict(SCREENROOM_EMBEDDING_BASE_URL='https://example.invalid/?token=secret')):
            with patch.dict(os.environ, dict(valid, **changed), clear=True):
                with self.assertRaisesRegex(RuntimeError, 'optional embedding'):
                    backend.validate_embeddings()
        with patch.dict(os.environ, {'SCREENROOM_EMBEDDING_API_KEY': ''}, clear=True):
            with self.assertRaises(RuntimeError):
                backend.validate_embeddings()

    def test_up_only_creates_missing_users_and_starts_empty_worker(self):
        calls = []
        with patch.object(backend, 'initialize_files'), patch.object(backend, 'bootstrap_garage'), \
                patch.object(backend, 'initialize_credentials') as credentials, patch.object(backend, 'provision_operator_role') as role, \
                patch.object(backend, 'initialize_playback_secrets') as playback_secrets, \
                patch.object(backend, 'wait'), patch.object(backend, 'compose', side_effect=lambda *a, **kw: calls.append(a)), \
                patch.object(sys, 'argv', ['backend', 'up']), patch.dict(os.environ, {}, clear=True):
            backend.main()
        self.assertFalse(any('screenroom-catalog-init' in call for call in calls))
        self.assertIn(('run', '--rm', '--no-deps', 'user', 'screenroom-user-init', '--create-missing', '/runtime/users.json'), calls)
        self.assertIn(('up', '-d', 'user', 'catalog', 'worker', 'playback-gateway', 'index-operator', 'edge'), calls)
        credentials.assert_called_once()
        playback_secrets.assert_called_once()
        role.assert_called_once()

    def test_wait_accepts_empty_catalog_and_writes_actual_selector(self):
        backend.initialize_files()
        identity = backend.read_config(self.runtime / 'identity.json')
        response = MagicMock()
        response.__enter__.return_value = response
        response.read.return_value = json.dumps(identity)
        def rpc(method, **kwargs):
            return {'user': {'id': backend.installation_user()}} if method.endswith('GetCurrentUser') else {}
        opener = MagicMock()
        opener.open.return_value = response
        with patch.object(backend, 'rpc', side_effect=rpc), patch.object(backend.urllib.request, 'build_opener', return_value=opener), \
                patch.object(backend.importlib.util, 'module_from_spec', return_value=SimpleNamespace(web_rpc=rpc)), \
                patch.object(backend.importlib.util, 'spec_from_file_location', return_value=SimpleNamespace(loader=SimpleNamespace(exec_module=lambda *_: None))):
            backend.wait()
        descriptor = backend.read_config(self.runtime / 'descriptor.json')
        self.assertEqual(descriptor['userID'], backend.installation_user())
        self.assertTrue(descriptor['ready'])
        self.assertTrue(descriptor['checks']['playback'])
        self.assertNotIn('embeddingModel', descriptor)

    def test_wait_budget_is_finite(self):
        for value in ('0', '901', '-1'):
            with patch.dict(os.environ, SCREENROOM_BACKEND_WAIT_SECONDS=value):
                with self.assertRaisesRegex(RuntimeError, 'between 1 and 900'):
                    backend.wait()

    def test_wait_cannot_publish_ready_until_private_playback_is_ready(self):
        backend.initialize_files()
        identity = backend.read_config(self.runtime / 'identity.json')
        before = {p: p.read_bytes() for p in self.runtime.iterdir() if p.is_file()}
        response = MagicMock()
        response.__enter__.return_value = response
        response.read.return_value = json.dumps(identity)
        def rpc(method, **kwargs):
            return {'user': {'id': backend.installation_user()}} if method.endswith('GetCurrentUser') else {}
        opener = MagicMock()
        opener.open.return_value = response
        with patch.object(backend, 'rpc', side_effect=rpc), \
                patch.object(backend.urllib.request, 'build_opener', return_value=opener), \
                patch.object(backend.importlib.util, 'module_from_spec', return_value=SimpleNamespace(web_rpc=rpc)), \
                patch.object(backend.importlib.util, 'spec_from_file_location', return_value=SimpleNamespace(loader=SimpleNamespace(exec_module=lambda *_: None))), \
                patch.object(backend, 'playback_ready', side_effect=RuntimeError('not ready')), \
                patch.object(backend.time, 'monotonic', side_effect=[0, 0, 1000, 1000]), \
                patch.object(backend.time, 'sleep'):
            with self.assertRaisesRegex(RuntimeError, 'readiness timed out'):
                backend.wait()
        self.assertFalse((self.runtime / 'descriptor.json').exists())
        self.assertEqual(before, {p: p.read_bytes() for p in before})

    def test_existing_garage_never_imports_or_rotates_keys(self):
        backend.initialize_files()
        calls = []
        def garage(*args, **kwargs):
            calls.append(args)
            return SimpleNamespace(returncode=0, stdout='Current cluster layout version: 1')
        with patch.object(backend, 'garage', side_effect=garage):
            backend.bootstrap_garage()
        self.assertFalse(any('import' in args or 'create' in args or 'assign' in args for args in calls))
        self.assertEqual(sum('--write' in args for args in calls), 1)

    def test_worker_timeout_stops_only_owned_oneoff(self):
        with patch.object(backend, 'bounded_run', side_effect=[subprocess.TimeoutExpired('double', 1), SimpleNamespace(returncode=0)]) as run:
            with self.assertRaises(subprocess.TimeoutExpired):
                backend.worker_command(['drain'], timeout=1)
        first, stop = run.call_args_list
        name = first.args[0][first.args[0].index('--name') + 1]
        self.assertTrue(name.startswith(backend.PROJECT + '-operation-'))
        self.assertEqual(stop.args[0], ['docker', 'stop', '--time', '15', name])

    def test_wrong_owner_and_existing_container_never_teardown(self):
        owner = self.runtime / 'owner'
        owner.write_text('another-project')
        with patch.object(backend, 'compose') as compose, patch.object(sys, 'argv', ['backend', 'down', '--owner-file', str(owner)]):
            with self.assertRaisesRegex(RuntimeError, 'owner mismatch'):
                backend.main()
            compose.assert_not_called()
        with patch.object(backend, 'run', return_value=SimpleNamespace(stdout='existing')), patch.object(sys, 'argv', ['backend', 'up', '--owner-file', str(owner)]):
            with self.assertRaisesRegex(RuntimeError, 'explicit attach'):
                backend.main()
        self.assertEqual(owner.read_text(), 'another-project')

    def test_credentials_init_only_repairs_empty_new_volume(self):
        with patch.object(backend, 'compose') as compose:
            backend.initialize_credentials()
        args = compose.call_args.args
        self.assertEqual(args[:8], ('run', '--rm', '--no-deps', '--user', '0:0', '--entrypoint', '/bin/sh', 'index-operator'))
        script = args[-1]
        self.assertIn('[ -z "$(ls -A "$d")" ]', script)
        self.assertIn('10001:10001:700', script)
        self.assertNotIn('-R', script)

    def test_operator_role_is_distinct_and_create_only(self):
        with patch.object(backend, 'sql') as sql:
            backend.provision_operator_role()
        query = sql.call_args.args[0]
        self.assertIn("rolname='operator_runtime'", query)
        self.assertIn('IN ROLE screenroom_operator', query)
        self.assertIn('IF NOT EXISTS', query)
        self.assertNotIn('ALTER ROLE', query)

    def test_compose_secret_permissions_roles_and_optional_embeddings(self):
        services = yaml.safe_load((backend.ROOT / 'compose.yaml').read_text())['services']
        self.assertFalse(services['catalog'].get('volumes'))
        for name in ('catalog', 'worker'):
            self.assertTrue(all(value is None for key, value in services[name]['environment'].items() if key.startswith('SCREENROOM_EMBEDDING_')))
        self.assertNotIn('SCREENROOM_JELLYFIN_CREDENTIAL_DIRECTORY', services['catalog']['environment'])
        for name in ('user', 'worker', 'index-operator', 'playback-gateway'):
            mount = next(v for v in services[name]['volumes'] if ':/run/screenroom/credentials' in v)
            self.assertEqual(mount.endswith(':ro'), name != 'index-operator')
            self.assertEqual(services[name]['environment']['SCREENROOM_JELLYFIN_CREDENTIAL_DIRECTORY'], '/run/screenroom/credentials')
        operator = services['index-operator']
        self.assertNotIn('profiles', operator)
        self.assertNotIn('ports', operator)
        self.assertIn('operator_runtime:', operator['environment']['DATABASE_URL'])
        self.assertIn('SCREENROOM_MIGRATION_DATABASE_URL', operator['environment'])
        dockerfile = (backend.ROOT / 'docker/backend/Dockerfile').read_text()
        self.assertIn('USER 10001:10001', dockerfile)
        self.assertNotIn('fixtures', dockerfile)
        self.assertIn('-p screenroom-playback-gateway', dockerfile)
        self.assertIn('/src/target/debug/screenroom-playback-gateway /usr/local/bin/', dockerfile)

    def test_playback_mounts_configuration_and_shutdown_are_private(self):
        config = yaml.safe_load((backend.ROOT / 'compose.yaml').read_text())
        services = config['services']
        gateway = services['playback-gateway']
        self.assertEqual(gateway['networks'], ['private'])
        self.assertNotIn('ports', gateway)
        self.assertEqual(gateway['stop_grace_period'], '75s')
        self.assertEqual(gateway['command'], ['screenroom-playback-gateway'])
        self.assertIn('http://127.0.0.1:50054/ready', gateway['healthcheck']['test'])
        self.assertEqual(gateway['volumes'], [
            'backend-jellyfin-credentials:/run/screenroom/credentials:ro',
            'backend-playback-secrets:/run/screenroom/playback'])
        for name, service in services.items():
            if name != 'playback-gateway':
                self.assertFalse(any('backend-playback-secrets:' in v for v in service.get('volumes', [])))
        env = gateway['environment']
        self.assertEqual(env['DATABASE_URL'], 'postgres://playback_runtime:playback-local-only@postgres/screenroom')
        self.assertEqual(env['SCREENROOM_MIGRATION_DATABASE_URL'], services['user']['environment']['SCREENROOM_MIGRATION_DATABASE_URL'])
        self.assertNotEqual(env['DATABASE_URL'], env['SCREENROOM_MIGRATION_DATABASE_URL'])
        self.assertEqual(env['SCREENROOM_PLAYBACK_API_ORIGIN'], services['catalog']['environment']['SCREENROOM_CATALOG_PUBLIC_ORIGIN'])
        self.assertEqual(env['SCREENROOM_PLAYBACK_GATEWAY_LISTEN'], '0.0.0.0:50054')
        origins = env['SCREENROOM_PLAYBACK_GATEWAY_CORS_ORIGINS']
        self.assertEqual(origins, services['user']['environment']['SCREENROOM_USER_CORS_ORIGINS'])
        self.assertEqual(origins, services['catalog']['environment']['SCREENROOM_CATALOG_CORS_ORIGINS'])
        self.assertNotIn('*', origins)
        self.assertEqual(env['SCREENROOM_PLAYBACK_SECRET_DIRECTORY'], '/run/screenroom/playback')
        self.assertIn('backend-playback-secrets', config['volumes'])

    def test_playback_bootstrap_is_additive_restricted_and_does_not_rotate(self):
        sql = (backend.ROOT / 'docker/backend/postgres.sql').read_text()
        self.assertIn("IF NOT EXISTS (SELECT FROM pg_roles WHERE rolname='screenroom_playback') THEN CREATE ROLE screenroom_playback NOLOGIN NOINHERIT;", sql)
        self.assertIn("IF NOT EXISTS (SELECT FROM pg_roles WHERE rolname='playback_runtime') THEN CREATE ROLE playback_runtime LOGIN INHERIT PASSWORD 'playback-local-only' IN ROLE screenroom_playback;", sql)
        self.assertNotIn('ALTER ROLE', sql)
        self.assertNotIn('DROP ', sql)
        self.assertNotIn('IN ROLE screenroom;', sql)

    def test_playback_volume_initialization_never_repairs_nonempty_or_existing_keys(self):
        self.prepare()
        before = {p: p.read_bytes() for p in self.runtime.iterdir() if p.is_file()}
        with patch.object(backend, 'compose') as compose:
            backend.initialize_playback_secrets()
        args = compose.call_args.args
        self.assertEqual(args[:8], ('run', '--rm', '--no-deps', '--user', '0:0', '--entrypoint', '/bin/sh', 'playback-gateway'))
        script = args[-1]
        self.assertIn('d=/run/screenroom/playback', script)
        self.assertNotIn('/run/screenroom/credentials', script)
        self.assertIn('[ -z "$(ls -A "$d")" ]', script)
        self.assertIn('10001:10001:700', script)
        self.assertNotIn('-R', script)
        self.assertEqual(before, {p: p.read_bytes() for p in before})

    def test_playback_readiness_is_private_and_teardown_allows_drain(self):
        with patch.object(backend, 'compose') as compose:
            backend.playback_ready()
        self.assertEqual(compose.call_args.args, ('exec', '-T', 'playback-gateway', 'curl',
            '--fail', '--silent', '--max-time', '3', 'http://127.0.0.1:50054/ready'))
        with patch.object(backend, 'compose') as compose, patch.object(sys, 'argv', ['backend', 'down']):
            backend.main()
        compose.assert_called_once_with('down', '--timeout', '75')

    def test_playback_edge_fixed_routes_streaming_and_query_redaction(self):
        config = yaml.safe_load((backend.ROOT / 'docker/backend/envoy.yaml').read_text())
        hcm = config['static_resources']['listeners'][0]['filter_chains'][0]['filters'][0]['typed_config']
        routes = hcm['route_config']['virtual_hosts'][0]['routes']
        prefixes = {r['match'].get('prefix'): r for r in routes}
        control = prefixes['/screenroom.playback_gateway.v1.PlaybackService/']
        self.assertEqual(control['route'], dict(cluster='playback', timeout='35s'))
        media = prefixes['/playback/v1/media/']
        self.assertEqual(media['route'], dict(cluster='playback', timeout='0s', idle_timeout='60s'))
        self.assertEqual(media['match']['headers'], [
            dict(name=':method', string_match=dict(safe_regex=dict(regex='GET|HEAD|OPTIONS')))])
        self.assertEqual(media['tracing']['overall_sampling'], dict(numerator=0))
        self.assertNotIn('tracing', hcm)
        self.assertIn('traceparent', media['request_headers_to_remove'])
        self.assertNotIn(':path', media['request_headers_to_remove'])
        self.assertNotIn('prefix_rewrite', media['route'])
        self.assertNotIn('regex_rewrite', media['route'])
        self.assertNotIn('retry_policy', media['route'])
        self.assertNotIn('response_headers_to_remove', media)
        self.assertEqual(hcm['request_headers_timeout'], '10s')
        self.assertEqual(hcm['stream_idle_timeout'], '60s')
        log = hcm['access_log'][0]['typed_config']['log_format']['json_format']
        self.assertEqual(log['path'], '%PATH(NQ:PATH)%')
        self.assertNotIn('%REQ(:PATH)%', json.dumps(hcm))
        self.assertNotIn('%REQ(X-ENVOY-ORIGINAL-PATH', json.dumps(hcm))
        self.assertFalse(any(r['match'].get('path') == '/ready' for r in routes))
        for prefix in ('/screenroom.catalog.v1.', '/artwork/', '/screenroom.user.v1.UserService/'):
            self.assertIn(prefix, prefixes)
        cluster = next(c for c in config['static_resources']['clusters'] if c['name'] == 'playback')
        endpoint = cluster['load_assignment']['endpoints'][0]['lb_endpoints'][0]['endpoint']['address']['socket_address']
        self.assertEqual(endpoint, dict(address='playback-gateway', port_value=50054))

    def test_playback_cors_is_owned_by_gateway_not_duplicated_at_edge(self):
        edge = (backend.ROOT / 'docker/backend/envoy.yaml').read_text()
        transport = (backend.ROOT / 'crates/screenroom-playback-gateway/src/transport.rs').read_text()
        self.assertNotIn('envoy.filters.http.cors', edge)
        self.assertNotIn('access-control-allow-origin', edge)
        for header in ('"range"', '"if-range"', '"content-range"', '"accept-ranges"',
                       '"etag"', '"last-modified"', '"x-screenroom-user-id"'):
            self.assertIn(header, transport)
        self.assertIn('.allow_origin(origins.clone())', transport)
        self.assertNotIn('.allow_credentials(true)', transport)

    def test_protocol_framing_and_pagination_bounds(self):
        trailer = b'grpc-status: 3\r\ngrpc-message: invalid\r\n'
        frame = b'\x80' + struct.pack('>I', len(trailer)) + trailer
        self.assertEqual(contract.grpc_status({}, frame), ('3', []))
        for body in (b'\x80', frame[:-1], b'\0\0\0\0\0', b'\x01\0\0\0\0'):
            with self.assertRaises(AssertionError):
                contract.grpc_status({}, body)
        with self.assertRaisesRegex(AssertionError, 'batch exceeds'):
            contract.pages('unused', 'items', size=2, transport=lambda *_: {'items': [1, 2, 3]})
        responses = iter(({'items': [1, 2], 'nextPageToken': 'next'}, {'items': [3]}))
        self.assertEqual(contract.pages('unused', 'items', size=2, transport=lambda *_: next(responses)), [1, 2, 3])

    def test_bounded_process_flood_and_timeout(self):
        with self.assertRaisesRegex(RuntimeError, 'output budget'):
            backend.bounded_run([sys.executable, '-c', "import os;\nwhile True: os.write(1,b'x'*4096)"], cwd=backend.ROOT, timeout=2, cap=4096, group=True)
        with self.assertRaises(subprocess.TimeoutExpired):
            backend.bounded_run([sys.executable, '-c', 'import time;time.sleep(60)'], cwd=backend.ROOT, timeout=0.2, group=True)


if __name__ == '__main__':
    unittest.main()
