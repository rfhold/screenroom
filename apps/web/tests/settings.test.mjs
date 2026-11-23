import { test } from 'node:test';
import assert from 'node:assert/strict';
import { readFile } from 'node:fs/promises';
import { createRequire } from 'node:module';
import { fileURLToPath } from 'node:url';
import * as core from '../../../target/client-core/wasm/screenroom_client_core.js';

const require = createRequire(import.meta.url);
const { build } = createRequire(require.resolve('vite'))('esbuild');

test('load migrates retired settings without losing scope or filters; saves remain strict and failures visible', async () => {
  await core.default({ module_or_path: await readFile(new URL('../../../target/client-core/wasm/screenroom_client_core_bg.wasm', import.meta.url)) });
  globalThis.settingsTestCore = core;
  const result = await build({
    entryPoints: [fileURLToPath(new URL('../src/lib/core/client.ts', import.meta.url))],
    bundle: true, write: false, format: 'esm', platform: 'node',
    define: { 'import.meta.env': JSON.stringify({ VITE_SCREENROOM_API_ENDPOINT: 'http://127.0.0.1:50051' }) },
    plugins: [{ name: 'settings-core', setup(build) {
      build.onResolve({ filter: /screenroom_client_core/ }, args => ({ path: args.path, namespace: 'fixture' }));
      build.onLoad({ filter: /.*/, namespace: 'fixture' }, ({ path }) => ({ contents: path.endsWith('?url')
        ? 'export default "unused.wasm";'
        : `export default async () => {}; export class Client {};
          export const { default_settings, normalize_saved_settings, validate_settings } = globalThis.settingsTestCore;` }));
    } }],
  });
  const adapter = await import(`data:text/javascript;base64,${Buffer.from(result.outputFiles[0].text).toString('base64')}`);
  const storage = new Map();
  let failWrites = false;
  globalThis.location = { origin: 'https://settings.test' };
  globalThis.localStorage = {
    getItem: key => storage.get(key) ?? null,
    setItem: (key, value) => { if (failWrites) throw Error('storage unavailable'); storage.set(key, value); },
  };
  const key = namespace => `screenroom.settings.v1:${JSON.stringify([location.origin, 'http://127.0.0.1:50051', '', namespace])}`;
  const expected = { ...core.default_settings(), scope: { source_id: 'source-uuid', library_id: 'opaque-library' }, source: 'all', types: { movie: false, series: true, episode: true }, methods: { title_only: true, text: false, fuzzy: true, semantic: false } };
  for (const [namespace, obsolete] of [['with-subtitle', { subtitle_preference: 'transcription' }], ['without-subtitle', {}]]) {
    storage.set('screenroom.settingsNamespace', namespace);
    storage.set(key(namespace), JSON.stringify({ ...expected, source: 'tmdb', ...obsolete }));
    assert.deepEqual(await adapter.loadSettings(), expected);
    await assert.rejects(adapter.saveSettings({ source: 'tmdb' }));
    assert.deepEqual(await adapter.loadSettings(), expected);
    await adapter.saveSettings({ source: 'jellyfin' });
    assert.deepEqual(JSON.parse(storage.get(key(namespace))), { ...expected, source: 'jellyfin' });
  }
  failWrites = true;
  await adapter.saveSettings({ source: 'all' });
  assert.deepEqual(await adapter.loadSettings(), expected);
  let warning;
  const unsubscribe = adapter.settingsWarning.subscribe(value => warning = value);
  assert.match(warning, /cannot be saved/);
  unsubscribe();
  delete globalThis.settingsTestCore;
});
