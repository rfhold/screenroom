#!/usr/bin/env python3
import importlib.util
import json
from pathlib import Path
import subprocess
import sys
import tempfile
from types import SimpleNamespace
import unittest
from unittest.mock import patch

sys.dont_write_bytecode = True
spec = importlib.util.spec_from_file_location('backend', Path(__file__).with_name('backend-lifecycle.py'))
backend = importlib.util.module_from_spec(spec)
spec.loader.exec_module(backend)


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

    def test_repeated_initialization_preserves_instance_and_keys(self):
        backend.initialize_files()
        before = {name: (self.runtime / name).read_bytes() for name in ('runtime.env', 'identity.json')}
        backend.initialize_files()
        self.assertEqual(before, {name: (self.runtime / name).read_bytes() for name in before})
        self.assertEqual((self.runtime / 'runtime.env').stat().st_mode & 0o777, 0o600)
        identity = json.loads(before['identity.json'])
        self.assertFalse(any('secret' in key.lower() or 'password' in key.lower() for key in identity))

    def test_existing_volumes_never_get_new_keys(self):
        with patch.object(backend, 'run', return_value=SimpleNamespace(stdout='owned-volume')):
            with self.assertRaisesRegex(RuntimeError, 'restore runtime.env'):
                backend.initialize_files()
        self.assertFalse((self.runtime / 'runtime.env').exists())

    def test_operator_bootstrap_files_are_never_rewritten(self):
        backend.initialize_files()
        installation = json.loads((self.runtime / 'installation.json').read_text())
        installation['sources'][0]['policy']['enabled'] = False
        users = json.loads((self.runtime / 'users.json').read_text())
        users[0]['display_name'] = 'Operator name'
        users[0]['links'] = []
        (self.runtime / 'installation.json').write_text(json.dumps(installation, indent=3))
        (self.runtime / 'users.json').write_text(json.dumps(users, indent=3))
        before = {name: (self.runtime / name).read_bytes() for name in ('installation.json', 'users.json')}
        backend.initialize_files()
        self.assertEqual(before, {name: (self.runtime / name).read_bytes() for name in before})

    def test_initial_notices_have_one_source_owner(self):
        backend.initialize_files()
        sources = json.loads((self.runtime / 'installation.json').read_text())['sources']
        notices = [notice for source in sources for notice in source['configuration']['notices']]
        self.assertEqual(len(notices), 7)
        self.assertEqual(len({notice['id'] for notice in notices}), 7)
        self.assertEqual(sources[0]['configuration']['notices'], [])

    def test_invalid_existing_config_is_rejected_not_replaced(self):
        backend.initialize_files()
        path = self.runtime / 'users.json'
        path.write_text('{}')
        with self.assertRaisesRegex(RuntimeError, 'invalid existing users.json'):
            backend.initialize_files()
        self.assertEqual(path.read_text(), '{}')

    def test_atomic_create_never_overwrites_a_winner(self):
        from concurrent.futures import ThreadPoolExecutor
        path = self.runtime / 'race.json'
        with ThreadPoolExecutor(max_workers=8) as executor:
            list(executor.map(lambda value: backend.create_json(path, {'winner': value}), range(20)))
        winner = json.loads(path.read_text())
        backend.create_json(path, {'winner': -1})
        self.assertEqual(json.loads(path.read_text()), winner)

    def test_subprocess_deadline_kills_descendants_and_reaps_child(self):
        import time
        path = self.runtime / 'child.pid'
        start = time.monotonic()
        with self.assertRaises(subprocess.TimeoutExpired):
            backend.bounded_run(['python3', '-c',
                "import subprocess,time,sys; p=subprocess.Popen(['sleep','60']); open(sys.argv[1],'w').write(str(p.pid)); time.sleep(60)", str(path)],
                cwd=backend.ROOT, timeout=0.5, group=True)
        self.assertLess(time.monotonic() - start, 3)
        pid = path.read_text()
        result = subprocess.run(['ps', '-o', 'stat=', '-p', pid], capture_output=True, text=True)
        self.assertTrue(not result.stdout.strip() or result.stdout.strip().startswith('Z'), result.stdout)

    def test_subprocess_output_flood_is_bounded(self):
        with self.assertRaisesRegex(RuntimeError, 'output budget'):
            backend.bounded_run(['python3', '-c', "import os;\nwhile True: os.write(1,b'x'*4096)"],
                cwd=backend.ROOT, timeout=2, cap=4096, group=True)

    def test_pages_reject_oversized_batches_and_follow_valid_continuations(self):
        spec = importlib.util.spec_from_file_location('contract', backend.ROOT / 'scripts/backend-contract.py')
        contract = importlib.util.module_from_spec(spec)
        spec.loader.exec_module(contract)
        with self.assertRaisesRegex(AssertionError, 'batch exceeds requested size'):
            contract.pages('unused', 'items', size=2,
                transport=lambda *_: {'items': [{'id': 'a'}, {'id': 'b'}, {'id': 'c'}]})
        requests = []
        responses = iter(({'items': [{'id': 'a'}, {'id': 'b'}], 'nextPageToken': 'next'},
                          {'items': [{'id': 'c'}]}))
        def fetch(method, request):
            self.assertEqual(method, 'unused')
            requests.append(request)
            return next(responses)
        self.assertEqual(contract.pages('unused', 'items', {'libraryId': 'movies'}, size=2, transport=fetch),
                         [{'id': 'a'}, {'id': 'b'}, {'id': 'c'}])
        self.assertEqual(requests, [{'libraryId': 'movies', 'page': {'size': 2, 'token': ''}},
                                    {'libraryId': 'movies', 'page': {'size': 2, 'token': 'next'}}])

    def test_notice_contract_rejects_duplicates_or_changed_content(self):
        spec = importlib.util.spec_from_file_location('contract', backend.ROOT / 'scripts/backend-contract.py')
        contract = importlib.util.module_from_spec(spec)
        spec.loader.exec_module(contract)
        notices = json.loads((backend.ROOT / 'fixtures/catalog/notices.json').read_text())
        public = [{('linkLabel' if key == 'link_label' else 'linkUrl' if key == 'link_url' else key): value
                   for key, value in notice.items()} for notice in notices]
        retrieved = json.loads((backend.ROOT / 'fixtures/catalog/tmdb.json').read_text())['retrievedAt']
        contract.check_notices({'notices': public, 'retrievedAt': retrieved})
        with self.assertRaises(AssertionError):
            contract.check_notices({'notices': public + public})
        with self.assertRaises(AssertionError):
            contract.check_notices({'notices': list(reversed(public)), 'retrievedAt': retrieved})
        with self.assertRaises(AssertionError):
            contract.check_notices({'notices': public, 'retrievedAt': '2026-01-01T00:00:00Z'})
        public[0]['text'] = 'changed'
        with self.assertRaises(AssertionError):
            contract.check_notices({'notices': public})

    def test_public_request_encoding_matches_current_proto(self):
        spec = importlib.util.spec_from_file_location('contract', backend.ROOT / 'scripts/backend-contract.py')
        contract = importlib.util.module_from_spec(spec)
        spec.loader.exec_module(contract)
        for method, request in (
            ('ListArtwork', {'scope': {'sourceId': 'mock-tmdb', 'libraryId': 'movies'},
                             'page': {'size': 201, 'token': 'opaque'}}),
            ('Search', {'query': 'dragon', 'source': 'tmdb', 'sourceId': 'mock-tmdb', 'libraryId': 'movies',
                        'types': {'movie': True, 'series': True, 'episode': True},
                        'methods': {'titleOnly': True, 'text': True, 'fuzzy': True, 'semantic': False},
                        'page': {'size': 2, 'token': 'opaque'}}),
        ):
            encoded = contract.encode_request(f'screenroom.catalog.v1.{method}Request', request)
            decoded = backend.bounded_run(['buf', 'convert', '.', '--type', f'screenroom.catalog.v1.{method}Request',
                '--from', '-#format=binpb', '--to', '-#format=json'], input=encoded, timeout=15, cwd=backend.ROOT).stdout
            expected = json.loads(json.dumps(request))
            if method == 'Search':
                del expected['methods']['semantic']  # ProtoJSON omits false scalar defaults.
            self.assertEqual(json.loads(decoded), expected)

    def test_grpc_web_business_status_and_frame_bounds(self):
        import struct
        spec = importlib.util.spec_from_file_location('contract', backend.ROOT / 'scripts/backend-contract.py')
        contract = importlib.util.module_from_spec(spec)
        spec.loader.exec_module(contract)
        self.assertEqual(contract.grpc_status({'grpc-status': '12'}, b''), ('12', []))
        trailer = b'grpc-status: 3\r\ngrpc-message: invalid\r\n'
        frame = b'\x80' + struct.pack('>I', len(trailer)) + trailer
        self.assertEqual(contract.grpc_status({}, frame), ('3', []))
        for malformed in (b'\x80', frame[:-1], b'\0\0\0\0\0', b'\x01\0\0\0\0'):
            with self.assertRaises(AssertionError):
                contract.grpc_status({}, malformed)

    def test_request_encoder_flood_and_hang_kill_the_owned_process_tree(self):
        import os
        spec = importlib.util.spec_from_file_location('contract', backend.ROOT / 'scripts/backend-contract.py')
        contract = importlib.util.module_from_spec(spec)
        spec.loader.exec_module(contract)
        executable = self.runtime / 'buf'
        pidfile = self.runtime / 'encoder-pids.json'
        run = contract.backend.bounded_run
        for mode in ('flood', 'hang'):
            executable.write_text(f'''#!{sys.executable}
import json, os, subprocess, time
child = subprocess.Popen(['sleep', '60'])
with open({str(pidfile)!r}, 'w') as output:
    json.dump([os.getpid(), child.pid], output)
if {mode!r} == 'hang':
    time.sleep(60)
while True:
    os.write(1, b'x' * 4096)
''')
            executable.chmod(0o755)
            def bounded(args, **kwargs):
                return run(args, **dict(kwargs, timeout=2 if mode == 'flood' else 0.5))
            expected = RuntimeError if mode == 'flood' else subprocess.TimeoutExpired
            with patch.dict(os.environ, PATH=str(self.runtime) + os.pathsep + os.environ['PATH']), \
                    patch.object(contract.backend, 'bounded_run', side_effect=bounded):
                with self.assertRaises(expected):
                    contract.encode_request('screenroom.catalog.v1.GetCatalogInfoRequest', {})
            for pid in json.loads(pidfile.read_text()):
                result = subprocess.run(['ps', '-o', 'stat=', '-p', str(pid)], capture_output=True, text=True, timeout=5)
                self.assertTrue(not result.stdout.strip() or result.stdout.strip().startswith('Z'), result.stdout)

    def test_existing_garage_does_not_import_or_rotate_keys(self):
        backend.initialize_files()
        calls = []

        def garage(*args, **kwargs):
            calls.append(args)
            return SimpleNamespace(returncode=0, stdout='Current cluster layout version: 1')

        with patch.object(backend, 'garage', side_effect=garage):
            backend.bootstrap_garage()
        self.assertFalse(any('import' in args or 'create' in args or 'assign' in args for args in calls))
        self.assertEqual(sum('--write' in args for args in calls), 1)

    def test_endpoint_alias_mismatch_fails_before_docker(self):
        import os
        env = dict(os.environ, SCREENROOM_API_ENDPOINT='http://127.0.0.1:50051',
                   SCREENROOM_E2E_API_ENDPOINT='http://127.0.0.1:50052')
        result = subprocess.run(['python3', 'scripts/backend-lifecycle.py', 'wait'],
                                cwd=backend.ROOT, env=env, capture_output=True, text=True)
        self.assertNotEqual(result.returncode, 0)
        self.assertIn('SCREENROOM_E2E_API_ENDPOINT must equal', result.stderr)


if __name__ == '__main__':
    unittest.main()
