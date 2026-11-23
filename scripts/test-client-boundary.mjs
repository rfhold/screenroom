import assert from 'node:assert/strict';
import { mkdtemp, mkdir, writeFile, readFile, rm, symlink } from 'node:fs/promises';
import { spawnSync } from 'node:child_process';
import { fileURLToPath } from 'node:url';
import os from 'node:os';
import path from 'node:path';
import { test } from 'node:test';
import { checkClientBoundary, inspectSource } from './check-client-boundary.mjs';

async function fixture(t, entries, setup) {
  const root = await mkdtemp(path.join(os.tmpdir(), 'screenroom-boundary-'));
  t.after(() => rm(root, { recursive: true, force: true }));
  for (const [file, text] of Object.entries(entries)) {
    await mkdir(path.dirname(path.join(root, file)), { recursive: true });
    await writeFile(path.join(root, file), text);
  }
  if (setup) await setup(root);
  return { ...await checkClientBoundary(root), root };
}

test('server models/mappers, protocol labels, navigation, placeholders and app icons pass', async t => {
  const result = await fixture(t, {
    'apps/apple/Sources/Model.swift': 'struct Art { let id: String }; let x = Art(id: record.name); Image(systemName: "house"); Text("Loading…")',
    'apps/android/ui/src/main/kotlin/Mapper.kt': 'data class Title(val name: String); val a = Art(record.id, record.file); val provider = if (record.provider == "tmdb") "TMDB" else "Jellyfin"; Icon(Icons.Default.Home, null)',
    'apps/linux/src/model.rs': 'struct CatalogItem { name: String }; let name = record.name; let icon = "go-previous-symbolic";',
    'apps/web/src/lib/core/models.ts': 'export type Art = { id: string }; const a = { title: record.title, name: record.name };',
    'apps/web/src/lib/demo/navigation.ts': 'export const routes = ["home", "search"];',
    'apps/web/src/Placeholder.svelte': '<div class="placeholder">{initials}</div><style>.placeholder { background: linear-gradient(red, blue) }</style>',
    'apps/web/static/favicon.svg': '<svg/>',
    'apps/web/static/icons/search.svg': '<svg/>',
    'apps/apple/Resources/iOS/Assets.xcassets/AppIcon.appiconset/icon-1024.png': 'icon',
    'apps/apple/Resources/tvOS/Assets.xcassets/App Icon & Top Shelf Image.brandassets/Top Shelf.imageset/image@1x.png': 'icon',
    'apps/android/tv/src/main/res/drawable-hdpi/tv_banner.png': 'icon',
    'apps/android/mobile/src/main/res/mipmap-mdpi/ic_launcher.png': 'icon',
    'apps/android/ui/src/main/res/drawable/search.xml': '<vector/>',
    'apps/linux/data/icons/hicolor/scalable/apps/tv.screenroom.svg': '<svg/>',
  });
  assert.deepEqual(result.violations, []);
});

test('Android vector provider branding cannot hide in XML resources', async t => {
  const result = await fixture(t, { 'apps/android/ui/src/main/res/drawable/brand.xml': '<vector/>' });
  assert.ok(result.violations.some(v => v.rule === 'bundled-media'));
});

const platforms = {
  apple: ['apps/apple/Sources/View.swift', 'apps/apple/Resources/Brand/logo.png', 'BundledImage(file: "tmdb.png")'],
  android: ['apps/android/ui/src/main/kotlin/View.kt', 'apps/android/ui/src/main/res/drawable/tmdb.png', 'context.assets.open("artwork/${art.file}")'],
  linux: ['apps/linux/src/view.rs', 'apps/linux/data/brand/logo.svg', 'include_bytes!("../data/artwork/tmdb.svg")'],
  web: ['apps/web/src/View.svelte', 'apps/web/static/brand/logo.svg', '<img src="/artwork/tmdb.svg" />'],
};
for (const [platform, [code]] of Object.entries(platforms)) {
  test(`${platform}: global footers are rejected without banning per-item metadata`, async t => {
    const result = await fixture(t, { [code]: 'ProviderAttribution(); provider_attribution(); "provider.attribution"' });
    assert.ok(result.violations.some(v => v.rule === 'client-footer'));
    assert.deepEqual(inspectSource(code, 'record.providers; "Imdb"; "Tmdb"; "source_supplied"; record.titleRights; getCatalogInfo()'), []);
  });
}
for (const [platform, [code, image, loader]] of Object.entries(platforms)) {
  test(`${platform}: bundled logos and direct image paths fail`, async t => {
    const result = await fixture(t, { [code]: loader, [image]: 'image' });
    assert.ok(result.violations.some(v => v.path === code && v.rule === 'local-artwork'));
    assert.ok(result.violations.some(v => v.path === image && v.rule === 'bundled-media'));
  });
  test(`${platform}: fixture-specific provenance and local welcome flow fail`, async t => {
    const result = await fixture(t, { [code]: '"Sprite Fright retains its 2021 release year"; "Welcome"; "Continue"; refreshCount++;' });
    for (const rule of ['fixture-data', 'welcome-mock', 'refresh-mock']) assert.ok(result.violations.some(v => v.rule === rule), rule);
  });
}

test('dead fixture JSON, legacy artwork directories and web data modules fail even when unused', async t => {
  const result = await fixture(t, {
    'apps/apple/Resources/Catalog/tmdb.json': '{}',
    'apps/android/ui/src/main/assets/artwork.json': '{}',
    'apps/linux/data/catalog.json': '{}',
    'apps/web/src/lib/demo/tmdb.json': '{}',
    'apps/web/src/lib/demo/catalog.ts': '',
    'apps/web/src/lib/demo/providers.ts': '',
    'apps/web/src/lib/demo/search.ts': '',
    'apps/web/static/artwork/unused.jpg': 'image',
  });
  assert.equal(result.violations.filter(v => v.rule === 'fixture-json').length, 4);
  assert.equal(result.violations.filter(v => v.rule === 'legacy-fixture-module').length, 3);
  assert.equal(result.violations.filter(v => v.rule === 'legacy-artwork-directory').length, 2);
});

test('sibling website, canonical fixtures, tests and generated outputs are not production clients', async t => {
  const files = [
    'website/static/artwork/tmdb.svg', 'fixtures/catalog/catalog.json', 'apps/web/e2e/fixture.ts',
    'apps/web/build/artwork/tmdb.svg', 'apps/web/src/lib/view.test.ts', 'apps/web/src/__fixtures__/catalog.ts',
    'apps/apple/Tests/Mock.swift', 'apps/apple/Sources/Tests/Mock.swift',
    'apps/linux/tests/e2e.rs', 'apps/linux/src/test.rs',
    'apps/android/ui/src/test/kotlin/Mock.kt', 'apps/android/ui/src/androidTest/kotlin/Mock.kt',
    'apps/android/ui/build/generated/Mock.kt', 'apps/web/src/generated/Mock.ts',
  ];
  const result = await fixture(t, Object.fromEntries(files.map(file => [file, '"Sintel"; "Welcome"; "Continue"; refreshCount++;'])));
  assert.deepEqual(result.violations, []);
});

test('source rules cover literal loaders, fabricated records, IDs, packaging and inline image data', () => {
  for (const text of [
    'Art("tmdb", "remote")', 'if (item.id == "sintel") special();',
    'Image("tmdb.svg")', 'UIImage(named: "poster.jpg")', 'NSImage(named: "tmdb.png")',
    'poster::picture("tmdb.svg", "logo", 88, 32)', 'new FileInputStream("logo.png")',
    '"data:image/png;base64,abc"', 'R.drawable.tmdb', 'resources.srcDir("../../fixtures/catalog")',
    'R.drawable.logo', '<img src="/images/brand.png" />', 'background: url("/poster.jpg")',
  ]) assert.ok(inspectSource('view', text).length > 0, text);
});

test('literal app icon paths and dynamic server image URLs are allowed', () => {
  assert.deepEqual(inspectSource('apps/web/src/app.html', '<link href="/favicon.svg"><img src="/icon-192.png"><img src={serverUrl}>'), []);
  assert.deepEqual(inspectSource('apps/web/src/View.svelte', '<img src="https://server.test/artwork/logo.svg">'), []);
});

test('comments do not trip source rules; locations identify the actual offending line', () => {
  assert.deepEqual(inspectSource('view', '// "Sintel"\n/* refreshCount */\n<!-- Welcome Continue -->'), []);
  const result = inspectSource('view', '\n\nArt("tmdb", "logo")');
  assert.equal(result.find(v => v.rule === 'fabricated-art').line, 3);
});

test('retired remote preferences, mock profiles and private APIs fail public client boundaries', () => {
  for (const text of ['get_preferences()', 'updatePreferences(value)', 'PreferencesService', 'mock_profile: "old"', 'mockProfile: "old"', '"x-screenroom-mock-profile"', '"screenroom.user.operator.v1.JellyfinUserService"', '"screenroom.user.internal.v1.PlaybackStateService"', 'ingestion_service_client::new()']) {
    assert.ok(inspectSource('apps/web/src/client.ts', text).length > 0, text);
  }
  assert.deepEqual(inspectSource('apps/web/src/client.ts', 'const user_id = settings.user_id; const scope = { source_id: "mock-jellyfin", library_id: "movies" }; default_settings(); validate_settings(value);'), []);
});

test('separate operator capability has typed native/WASM parity without private RPC or backend imports', async () => {
  const root = fileURLToPath(new URL('../', import.meta.url));
  const [native, wasm, models, ts, publicModels] = await Promise.all(['operator.rs', 'operator_wasm.rs', 'operator_models.rs', 'operator-models.ts', 'models.rs'].map(name => readFile(path.join(root, 'crates/screenroom-client-core/src', name), 'utf8')));
  for (const source of [native, wasm, models, ts]) assert.doesNotMatch(source, /screenroom_backend|sqlx|ingestion_service_client|playback_state_service_client/);
  for (const source of [native, wasm]) {
    for (const method of ['health', 'sources', 'libraries', 'discover_connection', 'save_connection', 'connections', 'indexing_runs', 'indexing_candidates', 'indexing_policies', 'indexing_configure', 'indexing_progress', 'indexing_start', 'indexing_run_status', 'indexing_execution', 'indexing_pause', 'indexing_resume', 'indexing_cancel', 'indexing_jobs', 'indexing_attempts', 'indexing_redrives', 'indexing_redrive', 'indexing_queue']) assert.match(source, new RegExp(`pub async fn ${method}\\b`));
    assert.doesNotMatch(source, /pub async fn (policies|runs|run_status|candidates|status|configure|preview|start|pause|resume|cancel)\b/);
    for (const method of ['new', 'configuration', 'configuration_namespace', 'cancel_all']) assert.match(source, new RegExp(`pub fn ${method}\\b`));
  }
  for (const name of [...models.matchAll(/pub (?:struct|enum) (Operator\w+)/g)].map(match => match[1])) {
    assert.match(ts, new RegExp(`(?:interface|type) ${name}\\b`));
    assert.doesNotMatch(publicModels, new RegExp(`\\b${name}\\b`));
  }
  assert.match(ts, /policy_version: bigint/);
  assert.match(wasm, /serialize_large_number_types_as_bigints\(true\)/);
  assert.match(native, /OperatorServiceClient::new\(transport\)/);
  assert.match(native, /\.redirect\(Redirect::Error\)/);
  assert.doesNotMatch(wasm, /\bfetch\b/);
  for (const source of [models, ts]) assert.doesNotMatch(source, /\bOperator(?:Selection|WorkBudget|Configuration|ConfigureRequest|PreviewRequest|Preview|Status|Policy|PolicyPage)\b|\btrigger\s*:/);
  for (const name of ['OperatorDiscoverConnectionRequest', 'OperatorSaveConnectionRequest']) {
    const prefix = models.slice(0, models.indexOf(`pub struct ${name} `));
    assert.equal([...prefix.matchAll(/#\[derive\(([^)]*)\)\]/g)].at(-1)?.[1], 'Clone, Deserialize');
    assert.match(models, new RegExp(`impl std::fmt::Debug for ${name}\\b[\\s\\S]*?REDACTED`));
  }
});

test('ordinary playback has native/WASM parity and no operator/provider bypass', async () => {
  const root = path.resolve(import.meta.dirname, '..');
  const [native, wasm, player] = await Promise.all([
    'crates/screenroom-client-core/src/playback.rs',
    'crates/screenroom-client-core/src/wasm.rs',
    'apps/web/src/lib/core/playback.ts',
  ].map(name => readFile(path.join(root, name), 'utf8')));
  for (const method of ['create_playback_session', 'get_playback_session', 'update_playback_session', 'report_playback_event', 'stop_playback_session']) {
    for (const source of [native, wasm]) assert.match(source, new RegExp(`pub async fn ${method}\\b`));
    assert.match(player, new RegExp(`client\\.${method}\\(`));
  }
  assert.doesNotMatch(player, /OperatorClient|fetch\(|XMLHttpRequest|localStorage|sessionStorage/);
  assert.match(native, /PlaybackServiceClient::new/);
});

test('canonical generated DTOs and native/WASM RPC surfaces stay public-only and in parity', async () => {
  const root = fileURLToPath(new URL('../', import.meta.url));
  const [native, wasm, rustModels, tsModels] = await Promise.all(['lib.rs', 'wasm.rs', 'models.rs', 'wasm-models.ts'].map(name => readFile(path.join(root, 'crates/screenroom-client-core/src', name), 'utf8')));
  for (const source of [native, wasm, rustModels, tsModels]) assert.doesNotMatch(source, /PreferencesService|\bPreferences\b|get_preferences|update_preferences|mock_profile|mockProfile|x-screenroom-mock-profile|screenroom\.(?:catalog|user)\.(?:operator|internal)/);
  for (const source of [native, wasm]) {
    for (const method of ['get_home', 'list_libraries', 'list_library_items', 'get_item', 'get_catalog_info', 'list_children', 'list_credits', 'list_artwork', 'search', 'get_search_capabilities', 'get_current_user', 'load_artwork']) assert.match(source, new RegExp(`pub async fn ${method}\\b`));
  }
  for (const source of [rustModels, tsModels]) {
    for (const type of ['User', 'SourceLibraryScope', 'LibraryPage', 'ArtworkPage', 'SearchPage', 'ClientSettings']) assert.match(source, new RegExp(`(?:struct|interface) ${type}\\b`));
    assert.match(source, /effective_methods/); assert.match(source, /degradation_reason/);
  }
  const generated = spawnSync('python3', ['scripts/generate-client-models.py', '--check'], { cwd: root, encoding: 'utf8' });
  assert.equal(generated.status, 0, generated.stderr);
});

for (const ancestor of ['apps', 'apps/web', 'apps/web/src', 'apps/apple/Resources', 'apps/linux/data', 'apps/android', 'apps/android/ui', 'apps/android/ui/src']) {
  test(`reject production symlink ${ancestor} before descending into its target`, async t => {
    const external = await mkdtemp(path.join(os.tmpdir(), 'screenroom-boundary-external-'));
    t.after(() => rm(external, { recursive: true, force: true }));
    for (const file of ['src/View.svelte', 'View.svelte', 'main/assets/tmdb.svg', 'src/main/assets/tmdb.svg']) {
      await mkdir(path.dirname(path.join(external, file)), { recursive: true });
      await writeFile(path.join(external, file), 'Sprite Fright');
    }
    const result = await fixture(t, {}, async root => {
      const link = path.join(root, ancestor);
      await mkdir(path.dirname(link), { recursive: true });
      await symlink(external, link, 'dir');
    });
    assert.equal(result.filesScanned, 0);
    assert.deepEqual(result.violations.map(v => [v.path, v.rule]), [[ancestor, 'unverifiable-link']]);
  });
}

test('unselected canonical, website, test and generated symlinks remain out of scope', async t => {
  const result = await fixture(t, {}, async root => {
    for (const file of ['website', 'fixtures', 'apps/web/e2e', 'apps/web/src/__fixtures__', 'apps/web/src/generated', 'apps/android/ui/src/test', 'apps/android/e2e']) {
      const link = path.join(root, file);
      await mkdir(path.dirname(link), { recursive: true });
      await symlink('/nonexistent-boundary-test-target', link, 'dir');
    }
  });
  assert.deepEqual(result.violations, []);
});

test('a symlink supplied as the audit root is rejected without following it', async t => {
  const { root } = await fixture(t, { 'actual/apps/web/src/View.svelte': 'Sprite Fright' }, async root => {
    await symlink(path.join(root, 'actual'), path.join(root, 'alias'), 'dir');
  });
  const result = await checkClientBoundary(path.join(root, 'alias'));
  assert.equal(result.filesScanned, 0);
  assert.deepEqual(result.violations.map(v => [v.path, v.rule]), [['.', 'unverifiable-link']]);
});

test('literal embedded navigation glyphs and app icons share the narrow resource allowlist', async t => {
  const code = 'apps/linux/src/main.rs';
  const text = 'include_bytes!("../data/icons/search.svg"); include_str!("../data/icons/home-symbolic.svg"); include_bytes!("../data/icons/hicolor/32x32/apps/tv.screenroom.png");';
  assert.deepEqual(inspectSource(code, text), []);
  const result = await fixture(t, {
    [code]: text,
    'apps/linux/data/icons/search.svg': '<svg/>',
    'apps/linux/data/icons/home-symbolic.svg': '<svg/>',
    'apps/linux/data/icons/hicolor/32x32/apps/tv.screenroom.png': 'icon',
  });
  assert.deepEqual(result.violations, []);
  assert.deepEqual(inspectSource(code, 'from_file("../data/icons/search.svg")'), []);
});

test('literal resource loaders still reject logos, unknown media and unaligned glyph paths', () => {
  for (const text of [
    'include_bytes!("../data/icons/tmdb.svg")',
    'include_bytes!("../data/icons/logo.svg")',
    'include_str!("../data/icons/unknown.svg")',
    'include_bytes!("../data/search.svg")',
    'include_bytes!("../../../../outside/icons/search.svg")',
    'from_file("../data/icons/logo.svg")',
    'context.assets.open("artwork/tmdb.svg")',
  ]) assert.ok(inspectSource('apps/linux/src/main.rs', text).some(v => v.rule === 'local-artwork'), text);
});

test('CLI returns zero for allowed resources and nonzero for branding violations', async t => {
  const checker = fileURLToPath(new URL('./check-client-boundary.mjs', import.meta.url));
  for (const [contents, status] of [['include_bytes!("../data/icons/search.svg")', 0], ['include_bytes!("../data/icons/logo.svg")', 1]]) {
    const { root } = await fixture(t, { 'apps/linux/src/main.rs': contents });
    const result = spawnSync(process.execPath, [checker, '--root', root], { encoding: 'utf8' });
    assert.equal(result.status, status, result.stderr);
    assert.match(result.stdout, /Static guard only, not runtime network proof/);
    if (status) assert.match(result.stderr, /local-artwork/);
  }
});
