#!/usr/bin/env python3
"""Read-only public transport checks for the ordinary installation, including empty catalogs."""
import importlib.util
import json
from pathlib import Path
import signal
import struct
import sys
import urllib.request

sys.dont_write_bytecode = True
spec = importlib.util.spec_from_file_location('backend', Path(__file__).with_name('backend-lifecycle.py'))
backend = importlib.util.module_from_spec(spec)
spec.loader.exec_module(backend)


def pages(method, field, request=None, size=2, transport=None):
    items, token, seen = [], '', set()
    while True:
        response = (transport or backend.rpc)(method, dict(request or {}, page=dict(size=size, token=token)))
        batch = response.get(field, [])
        assert len(batch) <= size, 'pagination batch exceeds requested size'
        items.extend(batch)
        assert len(items) <= 10000, 'pagination item budget exceeded'
        token = response.get('nextPageToken', '')
        if not token:
            return items
        assert token not in seen, 'pagination loop'
        seen.add(token)
        assert len(seen) < 512, 'pagination page budget exceeded'


def web_rpc(path, request=None, error=None):
    package, _, method = path.rpartition('/')
    namespace = package.rsplit('.', 1)[0]
    payload = encode_request(f'{namespace}.{method}Request', request or {})
    req = urllib.request.Request(backend.ORIGIN + '/' + path,
        data=b'\0' + struct.pack('>I', len(payload)) + payload,
        headers={'Content-Type': 'application/grpc-web+proto', 'X-Grpc-Web': '1',
                 'Origin': 'http://127.0.0.1:4173',
                 'X-Screenroom-User-Id': backend.installation_user()})
    with urllib.request.build_opener(urllib.request.ProxyHandler({})).open(req, timeout=15) as response:
        body = response.read(2 * 1024 * 1024 + 1)
        assert len(body) <= 2 * 1024 * 1024, 'RPC byte budget exceeded'
        assert response.headers['Access-Control-Allow-Origin'] == 'http://127.0.0.1:4173'
        status, messages = grpc_status(response.headers, body)
    assert status == str(error or 0), f'RPC status mismatch: {path}'
    if error:
        return
    assert len(messages) == 1, 'expected exactly one RPC response'
    decoded = backend.bounded_run(['buf', 'convert', '.', '--type', f'{namespace}.{method}Response',
        '--from', '-#format=binpb', '--to', '-#format=json'], input=messages[0], timeout=15,
        cwd=backend.ROOT).stdout
    return json.loads(decoded)


def encode_request(type_name, request):
    payload = json.dumps(request).encode()
    assert len(payload) <= 65536, 'test request byte budget exceeded'
    code = '''import subprocess, sys
with subprocess.Popen(['buf', 'convert', '.', '--type', sys.argv[1],
        '--from', '-#format=json', '--to', '-#format=binpb'],
        stdin=sys.stdin.buffer, stdout=subprocess.PIPE) as process:
    while chunk := process.stdout.read(4096):
        sys.stdout.write(chunk.hex())
        sys.stdout.flush()
    sys.exit(process.wait())
'''
    result = backend.bounded_run([sys.executable, '-c', code, type_name], input=payload,
        timeout=15, cap=256 * 1024, group=True, cwd=backend.ROOT)
    return bytes.fromhex(result.stdout)


def grpc_status(headers, body):
    status, messages = headers.get('grpc-status'), []
    offset = 0
    while offset < len(body):
        assert len(body) - offset >= 5, 'truncated gRPC-web frame'
        flag, length = body[offset], struct.unpack('>I', body[offset + 1:offset + 5])[0]
        offset += 5
        assert length <= len(body) - offset, 'truncated gRPC-web payload'
        payload = body[offset:offset + length]
        offset += length
        if flag == 128:
            trailers = dict(line.split(':', 1) for line in payload.decode('ascii').split('\r\n') if line)
            status = trailers.get('grpc-status', '').strip()
        else:
            assert flag == 0, 'unsupported gRPC-web frame'
            messages.append(payload)
    assert status is not None, 'missing gRPC business status'
    return status, messages


def check():
    for service, method in (
        ('catalog.v1.BrowseService', 'GetHome'),
        ('catalog.v1.CatalogService', 'GetCatalogInfo'),
        ('catalog.v1.BrowseService', 'ListLibraries'),
        ('catalog.v1.SearchService', 'GetSearchCapabilities'),
        ('user.v1.UserService', 'GetCurrentUser'),
    ):
        path = f'screenroom.{service}/{method}'
        native, browser = backend.rpc(path), web_rpc(path)
        assert native == browser, f'native/grpc-web mismatch: {path}'
        if method == 'GetCurrentUser':
            assert native.get('user', {}).get('id') == backend.installation_user(), 'installation user mismatch'
    for private in ('/screenroom.catalog.operator.v1.IngestionService/GetPolicy',
                    '/screenroom.user.operator.v1.JellyfinUserService/SetLink',
                    '/screenroom.user.internal.v1.PlaybackStateService/GetContinueWatching', '/ready'):
        import urllib.error
        try:
            urllib.request.build_opener(urllib.request.ProxyHandler({})).open(
                urllib.request.Request(backend.ORIGIN + private, data=bytes(5)), timeout=5)
            raise AssertionError('private route exposed')
        except urllib.error.HTTPError as error:
            assert error.code == 404
    return dict(nativeGrpc=True, grpcWeb=True, privateRoutesAbsent=True,
                userID=backend.installation_user())


if __name__ == '__main__':
    def deadline(*_):
        raise TimeoutError('whole contract deadline exceeded')
    signal.signal(signal.SIGALRM, deadline)
    signal.alarm(120)
    print(json.dumps(check()))
