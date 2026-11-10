#!/usr/bin/env python3
"""Read-only acceptance of the shared instance, via real public transports and artwork."""
import hashlib
import http.client
import importlib.util
import json
from collections import Counter
from datetime import datetime
from pathlib import Path
import struct
import subprocess
import sys
import signal
from urllib.parse import urlsplit
import uuid
import urllib.error
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


def check_notices(info):
    canonical = json.loads((backend.ROOT / 'fixtures/catalog/notices.json').read_text())
    actual = info.get('notices', [])
    assert len(actual) == len(canonical), 'catalog notices must have exactly seven canonical identities'
    assert len({notice['id'] for notice in actual}) == len(actual), 'duplicate catalog notice identities'
    normalize = lambda notice: tuple(notice.get(key, '') for key in ('id', 'section', 'title', 'text', 'linkLabel', 'linkUrl'))
    expected = [{('linkLabel' if key == 'link_label' else 'linkUrl' if key == 'link_url' else key): value
                 for key, value in notice.items()} for notice in canonical]
    assert list(map(normalize, actual)) == list(map(normalize, expected)), 'canonical notice content/order mismatch'
    retrieved = json.loads((backend.ROOT / 'fixtures/catalog/tmdb.json').read_text())['retrievedAt']
    assert datetime.fromisoformat(retrieved.replace('Z', '+00:00')).utcoffset() is not None
    assert info['retrievedAt'] == retrieved, 'provider retrieval date mismatch'
    return len(actual)


def web_rpc(path, request=None, error=None):
    package, _, method = path.rpartition('/')
    namespace = package.rsplit('.', 1)[0]
    payload = encode_request(f'{namespace}.{method}Request', request or {})
    req = urllib.request.Request(backend.ORIGIN + '/' + path,
        data=b'\0' + struct.pack('>I', len(payload)) + payload,
        headers={'Content-Type': 'application/grpc-web+proto', 'X-Grpc-Web': '1',
                 'Origin': 'http://127.0.0.1:4173',
                 'X-Screenroom-User-Id': '00000000-0000-0000-0000-000000000001'})
    with urllib.request.urlopen(req, timeout=15) as response:
        body = response.read(2 * 1024 * 1024 + 1)
        assert len(body) <= 2 * 1024 * 1024, 'RPC byte budget exceeded'
        assert response.headers['Access-Control-Allow-Origin'] == 'http://127.0.0.1:4173'
        status, messages = grpc_status(response.headers, body)
    assert status == str(error or 0), (path, status, body[-200:])
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
    # Stream binary as hex through the text-capture helper; own/reap the entire encoder group.
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


def rpc_error(path, request, code='InvalidArgument', number=3):
    try:
        backend.rpc(path, request)
    except subprocess.CalledProcessError as error:
        assert f'Code: {code}' in error.stderr, (path, error.stderr)
    else:
        raise AssertionError(f'{path} unexpectedly succeeded')
    web_rpc(path, request, error=number)


def check_item(actual, expected):
    for snake, camel in (('id', 'id'), ('title', 'title'), ('kind', 'kind'), ('year', 'year'),
                         ('subtitle', 'subtitle'), ('overview', 'overview'), ('tags', 'tags'),
                         ('colors', 'colors'), ('release_date', 'releaseDate'), ('added_order', 'addedOrder'),
                         ('parent_id', 'parentId'), ('provenance_notes', 'provenanceNotes'),
                         ('metadata_retrieved_at', 'metadataRetrievedAt')):
        default = [] if snake in ('tags', 'colors', 'provenance_notes') else None
        assert actual.get(camel, default) == expected.get(snake, default), (expected['id'], snake)
    for snake, camel in (('title_rights', 'titleRights'), ('metadata_rights', 'metadataRights')):
        for key in ('label', 'note', 'source', 'license_url', 'attribution'):
            assert actual[camel].get('licenseUrl' if key == 'license_url' else key, '') == expected[snake][key], (expected['id'], snake, key)
    for snake, camel in (('runtime', 'runtime'), ('rating', 'rating')):
        target, source = actual.get(camel), expected.get(snake)
        assert (target is None) == (source is None), (expected['id'], snake)
        if source is None:
            continue
        for key, value in source.items():
            name = {'minimum_seconds': 'minimumSeconds', 'maximum_seconds': 'maximumSeconds', 'retrieved_at': 'retrievedAt'}.get(key, key)
            observed = target.get(name, '' if isinstance(value, str) else None)
            if key == 'votes' and observed is not None:
                observed = int(observed)
            assert observed == value, (expected['id'], snake, key, observed, value)


def library_pagination_contract(fixture):
    path = 'screenroom.catalog.v1.BrowseService/ListLibraryItems'
    expected_ids = sorted(item['id'] for item in fixture['items'] if item['kind'] == 'Movie')
    assert len(expected_ids) > 2, 'canonical movie set must exercise continuation'
    native = None
    for transport in (backend.rpc, web_rpc):
        batches = []
        def fetch(method, request):
            response = transport(method, request)
            batches.append(response)
            return response
        items = pages(path, 'items', {'libraryId': 'movies'}, size=2, transport=fetch)
        assert len(batches[0].get('items', [])) == 2, 'first movie page must be full'
        assert [item['id'] for item in batches[0]['items']] == expected_ids[:2]
        assert len(batches) > 1 and batches[0].get('nextPageToken'), 'movie continuation missing'
        assert not batches[-1].get('nextPageToken'), 'movie traversal did not terminate'
        ids = [item['id'] for item in items]
        assert len(ids) == len(set(ids)), 'duplicate movie across library pages'
        # The real repository lists library items in public-ID order, not fixture addition order.
        assert ids == expected_ids, 'movie library membership/order mismatch'
        if native is None:
            native = items
        else:
            assert items == native, 'native/grpc-web movie pagination mismatch'


def credits_contract(fixture):
    path = 'screenroom.catalog.v1.CatalogService/ListCredits'
    canonical = fixture['credits']['nosferatu']
    tmdb = json.loads((backend.ROOT / 'fixtures/catalog/tmdb.json').read_text())['items']['nosferatu']
    assert len(canonical) == len(tmdb['cast']) + len(tmdb['crew'])
    signature = lambda credits: Counter((credit['name'], credit['role'], credit['department']) for credit in credits)
    expected = Counter((credit['name'], credit['role'], department)
                       for department in ('cast', 'crew') for credit in tmdb[department])
    assert signature(canonical) == expected
    assert len({credit['name'] for credit in canonical}) < len(canonical), 'repeated person roles must remain represented'
    assert any(role == 'Director' for _, role, _ in expected)
    for source in ('', 'mock-jellyfin', 'mock-tmdb'):
        request = {'itemId': 'nosferatu', 'scope': {'sourceId': source}}
        actual = pages(path, 'credits', request, size=3)
        assert signature(actual) == expected, (source, 'complete cast/crew including repeated person roles')
        if not source:
            assert pages(path, 'credits', request, size=3, transport=web_rpc) == actual
    assert pages('screenroom.catalog.v1.CatalogService/ListChildren', 'items', {'parentId': 'sintel'}) == []
    credit_artwork_contract(fixture)


def credit_artwork_contract(fixture):
    path = 'screenroom.catalog.v1.CatalogService/ListCredits'
    canonical = fixture['credits']['big-buck-bunny']
    signature = lambda credit: tuple(credit[key] for key in ('name', 'department', 'role'))
    expected = Counter(map(signature, canonical))
    assert all(count == 1 for count in expected.values()), 'ambiguous canonical credit role'
    manifest = {item['id']: item for item in json.loads((backend.ROOT / 'fixtures/catalog/artwork.json').read_text())}
    downloaded = {}
    for source in ('', 'mock-jellyfin', 'mock-tmdb'):
        request = {'itemId': 'big-buck-bunny', 'scope': {'sourceId': source}}
        actual = pages(path, 'credits', request, size=2)
        assert Counter(map(signature, actual)) == expected, (source, 'Big Buck Bunny credit roles')
        assert pages(path, 'credits', request, size=2, transport=web_rpc) == actual, (source, 'credit/artwork transport parity')
        by_role = {signature(credit): credit for credit in actual}
        people = {}
        portrait_ids = {}
        portraits = 0
        for credit in canonical:
            published = by_role[signature(credit)]
            person_id = str(uuid.UUID(published['id']))
            assert people.setdefault(credit['id'], person_id) == person_id, 'repeated person roles changed identity'
            expected_art = credit.get('artwork')
            if expected_art is None:
                continue
            portraits += 1
            art = published.get('artwork')
            assert art, (source, signature(credit), 'missing associated portrait')
            art_id = str(uuid.UUID(art['id']))
            for key in ('label', 'creator', 'license', 'source', 'license_url', 'kind'):
                assert art.get('licenseUrl' if key == 'license_url' else key, '') == expected_art[key], (source, signature(credit), key)
            prefix = f'{backend.ORIGIN}/artwork/{art_id}/revisions/'
            url = art['url']
            revision = url.removeprefix(prefix).removesuffix('/native')
            assert revision.isdecimal() and int(revision) > 0 and url == f'{prefix}{int(revision)}/native', 'invalid associated native artwork URL'
            assert portrait_ids.setdefault(expected_art['id'], (art_id, url)) == (art_id, url), 'repeated portrait roles changed artwork identity'
            source_art = manifest[expected_art['id']]
            filename = source_art.get('renderFile', source_art['file'])
            trusted = (backend.ROOT / 'fixtures/catalog/artwork' / filename).read_bytes()
            mime = {'.jpg': 'image/jpeg', '.png': 'image/png'}[Path(filename).suffix]
            if url not in downloaded:
                with urllib.request.urlopen(url, timeout=15) as response:
                    image = response.read(4 * 1024 * 1024 + 1)
                    assert len(image) <= 4 * 1024 * 1024, 'portrait byte budget exceeded'
                    assert response.headers['Content-Type'] == mime
                    downloaded[url] = image
            assert downloaded[url] == trusted, (source, signature(credit), 'associated portrait bytes differ')
        assert portraits == 5 and len(portrait_ids) == 4 and len(people) == len(set(people.values())), 'portrait roles/person identities collapsed'


def lexical_request(query, **extra):
    return dict(query=query, types={'movie': True, 'series': True, 'episode': True},
                methods={'text': True, 'fuzzy': True, 'semantic': False}, **extra)


def check_lexical(response, title_only=False):
    effective = response['effectiveMethods']
    assert effective.get('text') is True and effective.get('fuzzy') is True
    assert effective.get('semantic', False) is False
    assert effective.get('titleOnly', False) is title_only
    assert not response.get('degradationReason'), 'lexical search must not use embedding fallback'


def search_contract():
    path = 'screenroom.catalog.v1.SearchService/Search'
    for query, identity in (('Nosferatu', 'nosferatu'), ('nosferatu!!!', 'nosferatu'),
                            ('nosferat', 'nosferatu'), ('dragon', 'sintel'), ('mushroom', 'sprite-fright')):
        request = lexical_request(query)
        actual = backend.rpc(path, request)
        check_lexical(actual)
        assert identity in {item['id'] for item in actual.get('items', [])}, ('lexical query missing', query, identity)
        browser = web_rpc(path, request)
        check_lexical(browser)
        assert browser == actual, ('lexical native/browser parity', query)
    for provider in ('jellyfin', 'tmdb'):
        request = lexical_request('Sintel', source=provider, sourceId=f'mock-{provider}')
        actual = backend.rpc(path, request)
        check_lexical(actual)
        assert 'sintel' in {item['id'] for item in actual.get('items', [])}, provider
        assert web_rpc(path, request) == actual
    for provider, source in (('tmdb', 'mock-jellyfin'), ('jellyfin', 'mock-tmdb')):
        request = lexical_request('Sintel', source=provider, sourceId=source)
        actual = backend.rpc(path, request)
        check_lexical(actual)
        assert not actual.get('items'), 'contradictory provider/source must not leak results'
        assert web_rpc(path, request) == actual
    request = lexical_request('dragon')
    request['methods']['titleOnly'] = True
    actual = backend.rpc(path, request)
    check_lexical(actual, title_only=True)
    assert not actual.get('items')
    assert web_rpc(path, request) == actual
    # The production service deliberately rejects preprocessed-empty queries.
    rpc_error(path, lexical_request('   '))
    for field, value in (('types', {}), ('methods', {}), ('source', 'real-provider')):
        request = lexical_request('Sintel')
        request[field] = value
        rpc_error(path, request)


def pagination_errors():
    prefix = 'screenroom.catalog.v1.CatalogService/'
    for method, request in (('ListArtwork', {}), ('ListCredits', {'itemId': 'nosferatu'}),
                            ('ListChildren', {'parentId': 'beverly-hillbillies-s1'})):
        for page in ({'size': 201}, {'size': 2, 'token': 'garbage'}):
            rpc_error(prefix + method, dict(request, page=page))
    library = 'screenroom.catalog.v1.BrowseService/ListLibraryItems'
    for path, request in ((library, {'libraryId': 'movies'}),
                          ('screenroom.catalog.v1.BrowseService/ListLibraries', {}),
                          ('screenroom.catalog.v1.SearchService/Search', lexical_request('Sintel'))):
        for page in ({'size': 201}, {'size': 2, 'token': 'garbage'}):
            rpc_error(path, dict(request, page=page))
    first = backend.rpc(library, {'libraryId': 'movies', 'page': {'size': 1}})
    token = first['nextPageToken']
    rpc_error(library, {'libraryId': 'tv', 'page': {'size': 1, 'token': token}})
    rpc_error(library, {'libraryId': 'movies', 'sourceId': 'mock-jellyfin', 'page': {'size': 1, 'token': token}})
    rpc_error(prefix + 'ListArtwork', {'page': {'size': 1, 'token': token}})
    first = backend.rpc(prefix + 'ListCredits', {'itemId': 'nosferatu', 'page': {'size': 1}})
    token = first['nextPageToken']
    rpc_error(prefix + 'ListCredits', {'itemId': 'sintel', 'page': {'size': 1, 'token': token}})
    rpc_error(prefix + 'ListCredits', {'itemId': 'nosferatu', 'scope': {'sourceId': 'mock-tmdb'}, 'page': {'size': 1, 'token': token}})
    first = backend.rpc(prefix + 'ListChildren', {'parentId': 'beverly-hillbillies-s1', 'page': {'size': 1}})
    rpc_error(prefix + 'ListChildren', {'parentId': 'beverly-hillbillies', 'page': {'size': 1, 'token': first['nextPageToken']}})
    rpc_error(prefix + 'GetItem', {'id': 'missing'}, code='NotFound', number=5)


def semantic():
    notices = check_notices(backend.rpc('screenroom.catalog.v1.CatalogService/GetCatalogInfo')['info'])
    fixture = json.loads((backend.ROOT / 'fixtures/catalog/fixture.json').read_text())
    expected = fixture['items']
    for item in expected:
        actual = backend.rpc('screenroom.catalog.v1.CatalogService/GetItem', {'id': item['id']})['item']
        check_item(actual, item)
        if item['id'] in ('nosferatu', 'sprite-fright', 'beverly-hillbillies-s1', 'beverly-hillbillies-s1-e1'):
            assert web_rpc('screenroom.catalog.v1.CatalogService/GetItem', {'id': item['id']})['item'] == actual
        references = lambda entries: [tuple(entry.get(key, '') for key in ('provider', 'status', 'url')) for entry in entries]
        assert references(actual.get('providerReferences', [])) == references(item.get('provider_references', []))
        assert actual.get('provenanceNotes', []) == item.get('provenance_notes', [])
        if item.get('parent_id'):
            assert actual['parentId'] == item['parent_id']
    for parent in [item for item in expected if item['kind'] in ('Series', 'Season')]:
        children = pages('screenroom.catalog.v1.CatalogService/ListChildren', 'items', {'parentId': parent['id']}, 1)
        assert {item['id'] for item in children} == {item['id'] for item in expected if item.get('parent_id') == parent['id']}
    libraries = pages('screenroom.catalog.v1.BrowseService/ListLibraries', 'libraries', size=1)
    assert {'movies', 'tv'} <= {item['id'] for item in libraries}
    library_pagination_contract(fixture)
    home = backend.rpc('screenroom.catalog.v1.BrowseService/GetHome')
    assert any(item.get('progressPercent', 0) > 0 for row in home['rows'] for item in row.get('items', [])), 'mapped progress missing'
    current = backend.rpc('screenroom.user.v1.UserService/GetCurrentUser')['user']
    assert current['id'] == '00000000-0000-0000-0000-000000000001' and current.get('displayName')
    artwork = pages('screenroom.catalog.v1.CatalogService/ListArtwork', 'artwork', size=2)
    assert len({str(uuid.UUID(item['id'])) for item in artwork}) == len(artwork) == 21, 'complete source-scoped artwork identities missing'
    canonical = json.loads((backend.ROOT / 'fixtures/catalog/artwork.json').read_text())
    expected_labels = Counter({item['label']: 1 if item['id'] == 'tmdb' else 2 for item in canonical})
    assert Counter(item['label'] for item in artwork) == expected_labels, 'source-scoped artwork multiplicity mismatch'
    assert pages('screenroom.catalog.v1.CatalogService/ListArtwork', 'artwork', transport=web_rpc) == artwork
    scoped_ids = set()
    for provider in ('jellyfin', 'tmdb'):
        scoped = pages('screenroom.catalog.v1.CatalogService/ListArtwork', 'artwork', {'scope': {'sourceId': f'mock-{provider}'}}, size=3)
        labels = Counter(item['label'] for item in canonical if provider == 'tmdb' or item['id'] != 'tmdb')
        assert Counter(item['label'] for item in scoped) == labels
        ids = {item['id'] for item in scoped}
        assert scoped_ids.isdisjoint(ids), 'source-scoped artwork was semantically deduplicated'
        scoped_ids.update(ids)
    assert scoped_ids == {item['id'] for item in artwork}
    manifest = {item['label']: item for item in canonical}
    for art in artwork:
        assert art['url'].startswith(backend.ORIGIN + '/artwork/')
        source = manifest[art['label']]
        for key in ('label', 'creator', 'license', 'source', 'licenseUrl', 'kind'):
            assert art.get(key, '') == source['type' if key == 'kind' else key], (source['id'], key)
        for variant in ('native', 'original'):
            url = art['url'] if variant == 'native' else art['url'].rsplit('/', 1)[0] + '/original'
            filename = source.get('renderFile', source['file']) if variant == 'native' else source['file']
            trusted = (backend.ROOT / 'fixtures/catalog/artwork' / filename).read_bytes()
            mime = {'.jpg': 'image/jpeg', '.png': 'image/png', '.svg': 'image/svg+xml'}[Path(filename).suffix]
            if source['id'] == 'tmdb' and variant == 'native':
                assert len(trusted) == 10264
            with urllib.request.urlopen(urllib.request.Request(url, headers={'Origin': 'http://localhost:4173'}), timeout=15) as response:
                image = response.read(4 * 1024 * 1024 + 1)
                assert len(image) <= 4 * 1024 * 1024, 'artwork byte budget exceeded'
                assert image == trusted, (source['id'], variant, 'offline fixture bytes differ')
                assert hashlib.sha256(image).hexdigest() == response.headers['ETag'].strip('"')
                assert response.headers['Content-Type'] == mime
                assert response.headers['Cache-Control'] == 'private, max-age=0, must-revalidate'
                assert response.headers['Access-Control-Allow-Origin'] == 'http://localhost:4173'
                etag = response.headers['ETag']
            with urllib.request.urlopen(urllib.request.Request(url, method='HEAD'), timeout=15) as response:
                assert response.headers['ETag'] == etag
                assert int(response.headers['Content-Length']) == len(trusted)
                assert response.headers['Content-Type'] == mime
            try:
                urllib.request.urlopen(urllib.request.Request(url, headers={'If-None-Match': etag}), timeout=15)
                raise AssertionError('conditional artwork did not return 304')
            except urllib.error.HTTPError as error:
                assert error.code == 304
    first = artwork[0]
    with urllib.request.urlopen(first['url'], timeout=15) as response:
        body, headers = response.read(4 * 1024 * 1024 + 1), response.headers
        assert len(body) <= 4 * 1024 * 1024
    assert body and headers['Content-Type'].startswith('image/')
    with urllib.request.urlopen(urllib.request.Request(first['url'], method='HEAD'), timeout=15) as response:
        assert response.headers['ETag'] == headers['ETag']
        assert int(response.headers['Content-Length']) == len(body)
    conditional = urllib.request.Request(first['url'], headers={'If-None-Match': headers['ETag']})
    try:
        urllib.request.urlopen(conditional, timeout=15)
        raise AssertionError('conditional artwork did not return 304')
    except urllib.error.HTTPError as error:
        assert error.code == 304
    with urllib.request.urlopen(urllib.request.Request(first['url'], headers={'Origin': 'http://evil.example'}), timeout=15) as response:
        assert response.headers.get('Access-Control-Allow-Origin') is None
    artwork_rejections()
    credits_contract(fixture)
    search_contract()
    pagination_errors()
    return dict(items=len(expected), notices=notices, artwork=len(artwork), libraries=len(libraries), artworkSHA256=hashlib.sha256(body).hexdigest())


def artwork_rejections():
    endpoint = urlsplit(backend.ORIGIN)
    failures = []
    for target in ('/artwork/unknown.jpg', '/artwork/%2e%2e%2fCargo.toml', '/artwork/%2fetc%2fpasswd'):
        connection = http.client.HTTPConnection(endpoint.hostname, endpoint.port, timeout=10)
        try:
            connection.request('GET', target)  # Preserve the raw encoded HTTP target.
            response = connection.getresponse()
            if 200 <= response.status < 300:
                failures.append((target, response.status, response.getheader('grpc-status'), response.getheader('content-type')))
            assert len(response.read(65537)) <= 65536
        finally:
            connection.close()
    assert not failures, ('artwork errors must not return HTTP success', failures)


def web_parity():
    preflight = urllib.request.Request(backend.ORIGIN + '/screenroom.catalog.v1.BrowseService/GetHome', method='OPTIONS', headers={
        'Origin': 'http://127.0.0.1:4173', 'Access-Control-Request-Method': 'POST',
        'Access-Control-Request-Headers': 'content-type,x-grpc-web,x-screenroom-user-id'})
    with urllib.request.urlopen(preflight, timeout=5) as response:
        assert response.headers['Access-Control-Allow-Origin'] == 'http://127.0.0.1:4173'
        assert 'x-screenroom-user-id' in response.headers['Access-Control-Allow-Headers'].lower()
    for service, method in (
        ('catalog.v1.BrowseService', 'GetHome'),
        ('catalog.v1.CatalogService', 'GetCatalogInfo'),
        ('catalog.v1.BrowseService', 'ListLibraries'),
        ('catalog.v1.SearchService', 'GetSearchCapabilities'),
        ('user.v1.UserService', 'GetCurrentUser'),
    ):
        path = f'screenroom.{service}/{method}'
        decoded = web_rpc(path)
        if method == 'GetCatalogInfo':
            check_notices(decoded['info'])
        if method == 'GetSearchCapabilities':
            assert {'text', 'fuzzy', 'semantic'} <= set(decoded['methods'])
            assert {'tmdb', 'jellyfin'} <= set(decoded['sources'])
            assert decoded['maximumQueryLength'] == 8000
        assert decoded == backend.rpc(path), f'native/grpc-web mismatch: {path}'
    for private in ('/screenroom.catalog.operator.v1.IngestionService/GetPolicy',
                    '/screenroom.user.operator.v1.JellyfinUserService/SetLink',
                    '/screenroom.user.internal.v1.PlaybackStateService/GetContinueWatching', '/ready'):
        try:
            urllib.request.urlopen(urllib.request.Request(backend.ORIGIN + private, data=bytes(5)), timeout=5)
            raise AssertionError(f'private route exposed: {private}')
        except urllib.error.HTTPError as error:
            assert error.code == 404
    for service, method in (('BrowseService', 'ListLibraries'), ('CatalogService', 'GetCatalogInfo'),
                            ('SearchService', 'GetSearchCapabilities'), ('PreferencesService', 'GetPreferences')):
        target = f'/screenroom.client.v1.{service}/{method}'
        request = urllib.request.Request(backend.ORIGIN + target, data=bytes(5), headers={
            'Content-Type': 'application/grpc-web+proto', 'X-Grpc-Web': '1'})
        try:
            with urllib.request.urlopen(request, timeout=10) as response:
                body = response.read(65537)
                assert len(body) <= 65536
                status, _ = grpc_status(response.headers, body)
                assert status == '12', ('retired browser route unexpectedly succeeded', target, status)
        except urllib.error.HTTPError as error:
            assert error.code == 404, (target, error.code)
        native = backend.bounded_run(['curl', '--silent', '--show-error', '--http2-prior-knowledge',
            '--max-time', '10', '--include', '--header', 'content-type: application/grpc',
            '--request', 'POST', '--data-binary', '@-', backend.ORIGIN + target],
            input=bytes(5), timeout=15, cap=65536, cwd=backend.ROOT).stdout
        lines = native.splitlines()
        assert lines and lines[0].startswith('HTTP/2 '), ('native route check requires HTTP/2', target, native)
        statuses = [line.split(':', 1)[1].strip() for line in lines if line.lower().startswith('grpc-status:')]
        assert lines[0].split()[1] == '404' or statuses and statuses[-1] == '12', ('retired native route unexpectedly succeeded', target, native)


if __name__ == '__main__':
    def deadline(*_):
        raise TimeoutError('whole contract deadline exceeded')
    signal.signal(signal.SIGALRM, deadline)
    signal.alarm(120)
    result = semantic()
    web_parity()
    print(json.dumps(dict(result, nativeGrpc=True, grpcWeb=True, privateRoutesAbsent=True)))
