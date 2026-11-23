import { test } from 'node:test';
import assert from 'node:assert/strict';
import { mkdtemp, writeFile, rm, mkdir, readFile } from 'node:fs/promises';
import { spawnSync } from 'node:child_process';
import { fileURLToPath } from 'node:url';
import { rpcPath, isRpcPath } from '../e2e/rpc-paths.ts';

const cwd = fileURLToPath(new URL('../', import.meta.url));
const config = new URL('../playwright.config.ts', import.meta.url).href;
await mkdir(new URL('../e2e/build/', import.meta.url), { recursive: true });
const directory = await mkdtemp(fileURLToPath(new URL('../e2e/build/runner-test-', import.meta.url)));
const userID = '11111111-2222-4333-8444-555555555555';
const descriptor = `${directory}/descriptor.json`;
await writeFile(descriptor, JSON.stringify({ ready: true, userID }));
const env = { ...process.env, E2E_CATEGORY: 'all', SCREENROOM_BACKEND_DESCRIPTOR: descriptor };
delete env.SCREENROOM_E2E_JOURNEY;
delete env.SCREENROOM_USER_ID;
delete env.SCREENROOM_E2E_WEB_PORT;
function inspect(extra = {}) {
  return spawnSync(process.execPath, ['--input-type=module', '-e', `const {default:c}=await import(${JSON.stringify(config)});console.log(JSON.stringify({use:c.use,server:c.webServer[0]}));`], { cwd, env: { ...env, ...extra }, encoding: 'utf8' });
}

test('owned E2E server and browser share a non-development origin and installation user', () => {
  const result = inspect();
  assert.equal(result.status, 0, result.stderr);
  const value = JSON.parse(result.stdout);
  assert.equal(value.use.baseURL, 'http://127.0.0.1:4176');
  assert.equal(value.server.url, value.use.baseURL);
  assert.equal(value.server.env.SCREENROOM_E2E_WEB_PORT, '4176');
  assert.equal(value.server.env.VITE_SCREENROOM_USER_ID, userID);
  assert.equal(value.server.reuseExistingServer, false);
});
test('explicit port flows through readiness, base URL and child environment', () => {
  const result = inspect({ SCREENROOM_E2E_WEB_PORT: '4399' });
  assert.equal(result.status, 0, result.stderr);
  const value = JSON.parse(result.stdout);
  assert.equal(value.use.baseURL, 'http://127.0.0.1:4399');
  assert.equal(value.server.url, value.use.baseURL);
  assert.equal(value.server.env.SCREENROOM_E2E_WEB_PORT, '4399');
});
test('network waits use the fully qualified public RPC paths, not slash-delimited service names', async () => {
  for (const method of ['BrowseService/GetHome', 'BrowseService/ListLibraryItems', 'SearchService/Search', 'CatalogService/GetItem']) {
    const path = rpcPath(method);
    assert.equal(isRpcPath(path), true);
    assert.equal(path.endsWith(`/${method}`), false);
    assert.match(path, /^\/screenroom\.catalog\.v1\./);
  }
  const spec = await readFile(new URL('../e2e/home.spec.ts', import.meta.url), 'utf8');
  assert.ok(spec.includes("pathname === rpcPath('BrowseService/GetHome')"));
  assert.ok(spec.includes("pathname === rpcPath('BrowseService/ListLibraryItems')"));
});
test('reject invalid ports and mismatched installation selectors before startup', () => {
  for (const port of ['0', '80', '65536', '4183junk']) {
    const result = inspect({ SCREENROOM_E2E_WEB_PORT: port });
    assert.notEqual(result.status, 0);
    assert.match(result.stderr, /SCREENROOM_E2E_WEB_PORT/);
  }
  const mismatch = inspect({ SCREENROOM_USER_ID: 'aaaaaaaa-bbbb-4ccc-8ddd-eeeeeeeeeeee' });
  assert.notEqual(mismatch.status, 0);
  assert.match(mismatch.stderr, /does not match the installation descriptor/);
});
test('runner discovery lists captures without attaching a backend or clearing artifacts', async () => {
  const sentinel = new URL('../e2e/build/screenshots/home-desktop.png', import.meta.url);
  const before = await readFile(sentinel).catch(error => { if (error.code === 'ENOENT') return null; throw error; });
  const result = spawnSync(process.execPath, ['e2e/run.mjs', '--list'], { cwd, env: { ...env, SCREENROOM_BACKEND_DESCRIPTOR: `${directory}/missing.json`, SCREENROOM_E2E_API_ENDPOINT: 'http://127.0.0.1:1' }, encoding: 'utf8', timeout: 30_000 });
  assert.equal(result.status, 0, result.stderr);
  const report = JSON.parse(result.stdout);
  const specs = report.suites.flatMap(suite => suite.specs ?? []);
  assert.ok(specs.some(spec => spec.title.includes('real catalog showcase')));
  assert.equal(specs.find(spec => spec.title.includes('real catalog showcase')).tests.length, 2);
  // Discovery must exit before importing runtime preparation or screenshot cleanup.
  assert.ok(!result.stdout.includes('web-e2e-preflight'));
  const after = await readFile(sentinel).catch(error => { if (error.code === 'ENOENT') return null; throw error; });
  assert.deepEqual(after, before);
});
test('playback discovery selects exactly two viewport tests without backend attachment', () => {
  const result = spawnSync(process.execPath, ['e2e/run.mjs', '--list'], { cwd, env: { ...env, E2E_CATEGORY: 'happy-path', SCREENROOM_E2E_JOURNEY: 'playback', SCREENROOM_BACKEND_DESCRIPTOR: `${directory}/missing.json` }, encoding: 'utf8', timeout: 30_000 });
  assert.equal(result.status, 0, result.stderr);
  const report = JSON.parse(result.stdout);
  const specs = report.suites.flatMap(suite => suite.specs ?? []);
  assert.equal(specs.length, 1);
  assert.match(specs[0].title, /real selected title playback/);
  assert.deepEqual(specs[0].tests.map(test => test.projectName), ['desktop', 'mobile']);
  assert.ok(!result.stdout.includes('web-e2e-preflight'));
});
test('invalid journeys and playback categories fail before preparation, even on list', () => {
  for (const extra of [{ SCREENROOM_E2E_JOURNEY: 'wrong' }, { SCREENROOM_E2E_JOURNEY: 'playback', E2E_CATEGORY: 'all' }, { SCREENROOM_E2E_JOURNEY: 'playback', E2E_CATEGORY: 'retry' }]) {
    const result = spawnSync(process.execPath, ['e2e/run.mjs', '--list'], { cwd, env: { ...env, ...extra }, encoding: 'utf8', timeout: 30_000 });
    assert.notEqual(result.status, 0);
    assert.match(result.stderr, /Invalid SCREENROOM_E2E_JOURNEY|happy-path only/);
    assert.ok(!result.stdout.includes('web-e2e-preflight'));
  }
});
test('playback category intersection rejects zero tests before preparation', () => {
  const result = spawnSync(process.execPath, ['e2e/run.mjs', '--grep', 'no-such-playback-test'], { cwd, env: { ...env, E2E_CATEGORY: 'happy-path', SCREENROOM_E2E_JOURNEY: 'playback' }, encoding: 'utf8', timeout: 30_000 });
  assert.notEqual(result.status, 0);
  assert.ok(!result.stdout.includes('web-e2e-preflight'));
});
test('partial playback project selection fails before preparation', () => {
  const result = spawnSync(process.execPath, ['e2e/run.mjs', '--project=desktop', '--list'], { cwd, env: { ...env, E2E_CATEGORY: 'happy-path', SCREENROOM_E2E_JOURNEY: 'playback' }, encoding: 'utf8', timeout: 30_000 });
  assert.notEqual(result.status, 0);
  assert.match(result.stderr, /requires both desktop and mobile/);
  assert.ok(!result.stdout.includes('web-e2e-preflight'));
});
test.after(async () => { await rm(directory, { recursive: true, force: true }); });
