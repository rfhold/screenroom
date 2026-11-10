import test from 'node:test';
import assert from 'node:assert/strict';
import { isRpcPath, rpcPath, rpcRoute, rpcRoutes } from '../apps/web/e2e/rpc-paths.ts';

test('RPC paths classify each service and method in its domain namespace', () => {
  const services = {
    BrowseService: ['GetHome', 'ListLibraries', 'ListLibraryItems'],
    CatalogService: ['GetCatalogInfo', 'GetItem', 'ListChildren', 'ListCredits', 'ListArtwork'],
    SearchService: ['Search', 'GetSearchCapabilities'],
    UserService: ['GetCurrentUser'],
  };
  for (const [service, methods] of Object.entries(services)) {
    for (const method of methods) {
      const expected = `/screenroom.${service === 'UserService' ? 'user' : 'catalog'}.v1.${service}/${method}`;
      assert.equal(rpcPath(`${service}/${method}`), expected);
      assert.equal(rpcRoute(`${service}/${method}`), `**${expected}`);
      assert.equal(isRpcPath(expected), true);
      assert.equal(rpcRoutes.test(`http://127.0.0.1:50063${expected}`), true);
    }
  }
});

test('RPC allowlist rejects wrong namespaces, cross-service methods and path suffixes', () => {
  for (const pathname of [
    '/screenroom.catalog.v1.PreferencesService/GetPreferences',
    '/screenroom.user.v1.PreferencesService/GetPreferences',
    '/screenroom.user.operator.v1.JellyfinUserService/SetLink',
    '/screenroom.user.operator.v1.JellyfinUserService/ClearLink',
    '/screenroom.user.internal.v1.PlaybackStateService/GetContinueWatching',
    '/screenroom.catalog.operator.v1.IngestionService/GetPolicy',
    '/screenroom.catalog.operator.v1.IngestionService/UpdatePolicy',
    '/screenroom.catalog.operator.v1.IngestionService/StartRun',
    '/screenroom.catalog.operator.v1.IngestionService/GetRun',
    '/screenroom.catalog.operator.v1.IngestionService/CancelRun',
    '/screenroom.user.v1.BrowseService/GetHome',
    '/screenroom.catalog.v1.BrowseService/UpdatePreferences',
    '/screenroom.catalog.v1.UnknownService/GetHome',
    '/screenroom.other.v1.CatalogService/GetItem',
    '/screenroom.%63atalog.v1.CatalogService/GetItem',
    '/screenroom.catalog.v1.CatalogService%2FGetItem',
    '/screenroom.catalog.v1.CatalogService/GetItem%2Fextra',
    `${rpcPath('CatalogService/GetItem')}/extra`,
    `${rpcPath('CatalogService/GetItem')}?extra=true`,
    `/other${rpcPath('CatalogService/GetItem')}`,
  ]) {
    assert.equal(isRpcPath(pathname), false, pathname);
    assert.equal(rpcRoutes.test(`http://127.0.0.1:50063${pathname}`), false, pathname);
  }
  assert.throws(() => rpcPath('BrowseService/UpdatePreferences'), /Unknown RPC/);
  assert.equal(rpcRoutes.test('http://127.0.0.1:50063/screenroom.catalog.v1.PreferencesService/GetPreferences'), false);
  assert.equal(rpcRoutes.test(`http://127.0.0.1:50063${rpcPath('CatalogService/GetItem')}/extra`), false);
});
