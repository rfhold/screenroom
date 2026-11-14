import { mkdtemp, mkdir, readFile, writeFile, readdir, chmod } from 'node:fs/promises';
import { spawn } from 'node:child_process';
import { createHash, randomUUID } from 'node:crypto';
import path from 'node:path';
import assert from 'node:assert/strict';
import http from 'node:http';

const root = process.cwd();
await mkdir('build/backend-validation', { recursive: true });
const base = await mkdtemp(path.join(root, 'build/backend-validation/orchestration-'));
const userID = randomUUID();
const flows = ['home', 'sidebar', 'library', 'search', 'detail'];
const png = Buffer.from('iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAQAAAC1HAwCAAAAC0lEQVR42mP8/x8AAwMCAO+/l9sAAAAASUVORK5CYII=', 'base64');
const digest = bytes => createHash('sha256').update(bytes).digest('hex');
async function put(file, contents) {
  await mkdir(path.dirname(file), { recursive: true });
  await writeFile(file, contents);
}
async function tree(dir, prefix = '') {
  const result = {};
  for (const entry of await readdir(dir, { withFileTypes: true })) {
    const name = path.join(prefix, entry.name);
    if (entry.isDirectory()) Object.assign(result, await tree(path.join(dir, entry.name), name));
    else result[name] = digest(await readFile(path.join(dir, entry.name)));
  }
  return result;
}
const caller = http.createServer((req, res) => res.end('{}'));
await new Promise(resolve => caller.listen(0, '127.0.0.1', resolve));
const endpoint = `http://127.0.0.1:${caller.address().port}`;
const cases = [
  { name: 'default-40', count: 40 },
  { name: 'explicit-45', mac: '1', count: 45 },
  { name: 'external-server', count: 40, external: true },
  { name: 'suite-failure', failure: 'suite' },
  { name: 'invalid-png', failure: 'png' },
  { name: 'duplicate-attachment', failure: 'duplicate' },
  { name: 'export-failure', failure: 'export' },
  { name: 'export-interrupt', failure: 'export-int' },
  { name: 'interrupt', signal: 'SIGINT' },
  { name: 'terminate', signal: 'SIGTERM', external: true },
  { name: 'category-default-all', args: [], category: 'all', count: 0 },
  { name: 'linux-only', args: ['linux'], families: ['linux'], category: 'all', count: 0 },
  { name: 'backend-failure', args: ['linux'], backendFailure: true },
  { name: 'category-cli-happy', args: ['--category', 'happy-path'], category: 'happy-path', count: 0 },
  { name: 'category-cli-retry', args: ['--category', 'retry'], category: 'retry', count: 0 },
  { name: 'category-env-regression', args: [], categoryEnv: 'regression', category: 'regression', count: 0 },
  { name: 'category-env-all-export', categoryEnv: 'all', category: 'all', count: 40 },
  { name: 'category-cli-over-env', args: ['--update-showcase', '--category', 'happy-path'], categoryEnv: 'retry', category: 'happy-path', count: 40 },
  { name: 'category-cli-over-invalid-env', args: ['--update-showcase', '--category', 'all'], categoryEnv: 'unknown', category: 'all', count: 40 },
  { name: 'category-unknown-cli', args: ['--category', 'unknown'], reject: /Invalid E2E category/ },
  { name: 'category-empty-cli', args: ['--category', ''], reject: /Invalid E2E category/ },
  { name: 'category-missing-cli', args: ['--category'], reject: /--category requires a value/ },
  { name: 'category-unknown-env', args: [], categoryEnv: 'unknown', reject: /Invalid E2E category/ },
  { name: 'category-empty-env', args: [], categoryEnv: '', reject: /Invalid E2E category/ },
  { name: 'category-retry-export', categoryEnv: 'retry', reject: /Showcase export requires category happy-path or all/ },
  { name: 'category-regression-export', args: ['--update-showcase', '--category', 'regression'], mac: '1', reject: /Showcase export requires category happy-path or all/ },
  { name: 'unknown-family', args: ['unknown'], reject: /Unknown E2E family or option/ }
];
const evidence = [];
const prefixes = new Set();
try {
  for (const test of cases) {
    const siblings = test.families ?? ['apple', 'android', 'linux', 'web'];
    const cwd = path.join(base, test.name);
    await mkdir(path.join(cwd, 'control'), { recursive: true });
    await put(path.join(cwd, 'scripts/e2e.sh'), await readFile('scripts/e2e.sh'));
    await put(path.join(cwd, 'scripts/e2e-cleanup.mjs'), await readFile('scripts/e2e-cleanup.mjs'));
    await put(path.join(cwd, 'scripts/e2e-group.mjs'), await readFile('scripts/e2e-group.mjs'));
    await put(path.join(cwd, 'scripts/backend.sh'), `#!/bin/bash
set -eu
mkdir -p build/backend-runtime/screenroom-local
case "$1" in
 status) exit 1 ;;
 up|wait)
  echo "$1" >> backend-events
  if [ "$1" = up ]; then echo owned > "$3"; fi
  if [ "${'${FAIL_BACKEND:-0}'}" = 1 ]; then exit 17; fi
  echo '{"instanceID":"same-stack","project":"screenroom-local","bootstrapRevision":"installation-v1","publicOrigin":"http://127.0.0.1:50051","clientNetwork":"screenroom-local_client","userID":"${userID}","ready":true}' > build/backend-runtime/screenroom-local/descriptor.json ;;
 down) echo down >> backend-events ;;
esac
`);
    const website = path.join(cwd, 'website/static/showcase');
    await put(path.join(website, 'sentinel.txt'), 'caller-owned capture');
    for (const profile of ['ios', 'ipados', 'tvos', 'macos', 'android', 'android-tv', 'linux']) {
      for (const flow of flows) await put(path.join(website, profile, `${flow}.png`), 'previous capture');
    }
    for (const profile of ['desktop', 'mobile']) for (const flow of flows) {
      await put(path.join(website, 'web', `${profile}${flow === 'home' ? '' : `-${flow}`}.png`), 'previous capture');
      await put(path.join(cwd, 'apps/web/e2e/build/screenshots', `${flow}-${profile}.png`), png);
    }
    for (const profile of ['mobile', 'tv']) for (const flow of flows) await put(path.join(cwd, 'apps/android/build/e2e', `${profile}-${flow}.png`), png);
    for (const flow of flows) await put(path.join(cwd, 'apps/linux/build/e2e', `${flow}.png`), png);
    for (const profile of ['ios', 'ipad', 'tvos', 'macos']) {
      const attachments = flows.map(flow => ({ isAssociatedWithFailure: false, suggestedHumanReadableName: `${flow}-success_0.png`, exportedFileName: `${flow}.png` }));
      if (test.failure === 'duplicate' && profile === 'ios') attachments.push({ ...attachments[0] });
      for (const flow of flows) await put(path.join(cwd, 'apple-fixtures', profile, 'screenshots', `${flow}.png`), png);
      await put(path.join(cwd, 'apple-fixtures', profile, 'screenshots/manifest.json'), JSON.stringify([{ attachments }]));
    }
    const helper = `import fs from 'node:fs';
const [kind, ...profiles] = process.argv.slice(2);
const event = (state, profile = kind) => fs.appendFileSync('events.jsonl', JSON.stringify({kind, profile, state, time:Date.now(), pid:process.pid, endpoint:process.env.SCREENROOM_API_ENDPOINT, webEndpoint:process.env.SCREENROOM_E2E_API_ENDPOINT, userID:process.env.SCREENROOM_USER_ID, prepared:process.env.SCREENROOM_CORE_PREPARED, prefix:process.env.SCREENROOM_E2E_PROFILE_PREFIX})+'\\n');
event('start');
fs.appendFileSync('categories.jsonl', JSON.stringify({kind, category:process.env.E2E_CATEGORY})+'\\n');
if (!kind.startsWith('prepare')) {
  if (kind === 'apple') { fs.mkdirSync(process.env.E2E_APPLE_RUN_DIR); fs.writeFileSync(process.env.E2E_APPLE_RUN_DIR+'/test.log','retained-diagnostic'); }
  if (!profiles.includes('macos')) {
    event('ready');
    fs.writeFileSync('control/'+kind+'.ready', 'ready');
    const deadline = Date.now() + 20000;
    while (!fs.existsSync('control/'+kind+'.release')) {
      if (Date.now() >= deadline) throw new Error('Timed out waiting for '+kind+' release');
      await new Promise(resolve => setTimeout(resolve, 20));
    }
    event('released');
  }
  for (const profile of profiles) {
    event('profile-start',profile);
    if (kind === 'apple') fs.cpSync('apple-fixtures/'+profile,process.env.E2E_APPLE_RUN_DIR+'/'+profile,{recursive:true});
    event('profile-end',profile);
  }
}
event('end');
if (kind === 'linux' && ${test.failure === 'suite'}) process.exit(17);
`;
    await put(path.join(cwd, 'suite.mjs'), helper);
    for (const [file, contents] of Object.entries({
      'apps/apple/scripts/prepare-core.sh': 'node suite.mjs prepare-apple',
      'apps/android/scripts/build-core.sh': 'node suite.mjs prepare-android',
      'apps/web/scripts/prepare-core.mjs': "import '../../../suite.mjs';",
      'apps/apple/scripts/e2e.sh': 'node suite.mjs apple ${1:-ios ipad tvos}',
      'apps/android/scripts/e2e.sh': 'timeout -k 2s 30s node suite.mjs android mobile tv',
      'apps/linux/tests/e2e/run.sh': 'node suite.mjs linux linux',
      'bin/pnpm': 'if [ "${1-}" = --version ]; then echo 12.18.0; else node suite.mjs web desktop mobile; fi',
      'bin/cargo': 'echo "Root orchestration must not build a private backend" >&2; exit 99',
      'bin/docker': 'echo "Isolated runner diagnostics stub"'
    })) await put(path.join(cwd, file), file.endsWith('.mjs') ? contents : `#!/bin/bash\nset -eu\n${contents}\n`);
    // The web preparation stub deliberately records a preparation event, not a suite.
    await put(path.join(cwd, 'apps/web/scripts/prepare-core.mjs'), "process.argv.push('prepare-web'); await import('../../../suite.mjs');");
    for (const name of ['bin/pnpm', 'bin/cargo', 'bin/docker']) await chmod(path.join(cwd, name), 0o755);
    if (test.failure?.startsWith('export')) {
      await put(path.join(cwd, 'bin/cp'), `#!/bin/bash
if [[ "\${2-}" == website/static/showcase/* ]] && [[ "\${1-}" == */export/* ]] && [ ! -f export-triggered ]; then
  touch export-triggered
  /bin/cp "$@"
  ${test.failure === 'export-int' ? 'kill -INT "$PPID"' : ''}
  exit 19
fi
exec /bin/cp "$@"
`);
      await chmod(path.join(cwd, 'bin/cp'), 0o755);
    }
    if (test.failure === 'png') await put(path.join(cwd, 'apps/web/e2e/build/screenshots/detail-mobile.png'), 'not PNG');
    const before = await tree(website);
    const env = { ...process.env, PATH: `${cwd}/bin:${process.env.PATH}`, E2E_MACOS: test.mac ?? '0' };
    for (const key of ['E2E_APPLE_RUN_DIR', 'E2E_CATEGORY', 'SCREENROOM_API_ENDPOINT', 'SCREENROOM_E2E_API_ENDPOINT', 'SCREENROOM_CORE_PREPARED', 'SCREENROOM_BACKEND_ATTACH', 'SCREENROOM_USER_ID', 'SCREENROOM_BACKEND_PROJECT', 'SCREENROOM_BACKEND_DESCRIPTOR']) delete env[key];
    if (test.backendFailure) env.FAIL_BACKEND = '1';
    if ('categoryEnv' in test) env.E2E_CATEGORY = test.categoryEnv;
    if (test.external) { env.SCREENROOM_API_ENDPOINT = endpoint; env.SCREENROOM_BACKEND_ATTACH = '1'; }
    const child = spawn('bash', ['scripts/e2e.sh', ...(test.args ?? ['--update-showcase'])], { cwd, env, stdio: ['ignore', 'pipe', 'pipe'] });
    let stdout = '', stderr = '', exited = false, protocolError;
    child.stdout.on('data', chunk => {
      stdout += chunk;
    });
    child.stderr.on('data', chunk => { stderr += chunk; });
    const exit = new Promise(resolve => child.on('exit', code => { exited = true; resolve(code); }));
    const protocolEvents = [];
    const release = async names => {
      for (const name of names) await put(path.join(cwd, 'control', `${name}.release`), 'release');
    };
    const until = async (predicate, label) => {
      const deadline = Date.now() + 15000;
      while (!await predicate()) {
        if (exited) throw new Error(`${test.name}: root exited before ${label}`);
        if (Date.now() >= deadline) throw new Error(`${test.name}: timed out waiting for ${label}`);
        await new Promise(resolve => setTimeout(resolve, 20));
      }
      protocolEvents.push({ state: label, time: Date.now() });
    };
    const protocol = test.reject || test.backendFailure ? Promise.resolve() : (async () => {
      await until(async () => {
        const ready = await readdir(path.join(cwd, 'control'));
        return siblings.every(name => ready.includes(`${name}.ready`));
      }, 'all-siblings-ready');
      if (test.name === 'default-40') {
        const second = spawn('bash', ['scripts/e2e.sh', 'linux'], { cwd, env, stdio: ['ignore', 'ignore', 'pipe'] });
        let error = '';
        second.stderr.on('data', chunk => { error += chunk; });
        assert.equal(await new Promise(resolve => second.on('exit', resolve)), 1);
        assert.match(error, /Another root E2E owner/);
      }
      if (test.signal) {
        protocolEvents.push({ state: test.signal, time: Date.now() });
        child.kill(test.signal);
        return;
      }
      await release(['linux']);
      protocolEvents.push({ state: 'linux-released', time: Date.now() });
      await until(() => stdout.includes('E2E finished: linux '), 'linux-completion-reported');
      await release(siblings.filter(name => name !== 'linux'));
      protocolEvents.push({ state: 'other-siblings-released', time: Date.now() });
    })().catch(async error => {
      protocolError = error;
      await release(siblings);
      child.kill('SIGTERM');
    });
    const timer = setTimeout(() => child.kill('SIGTERM'), 60000);
    const code = await exit;
    await protocol;
    clearTimeout(timer);
    await put(path.join(cwd, 'run.log'), stdout + stderr);
    await put(path.join(cwd, 'control/protocol.json'), JSON.stringify(protocolEvents, null, 2));
    if (protocolError) throw protocolError;
    const after = await tree(website);
    if (test.reject) {
      assert.equal(code, 2, test.name);
      assert.match(stderr, test.reject);
      assert.deepEqual(after, before);
      for (const absent of ['build', 'events.jsonl', 'categories.jsonl']) {
        await assert.rejects(readdir(path.join(cwd, absent)), { code: 'ENOENT' }, `${test.name}: created ${absent}`);
      }
      evidence.push({ scenario: test.name, exitCode: code, rejectedBeforePreparation: true });
      console.log(`PASS ${test.name}`);
      continue;
    }
    if (test.backendFailure) {
      assert.notEqual(code, 0);
      assert.deepEqual(after, before);
      assert.equal(await readFile(path.join(cwd, 'backend-events'), 'utf8'), 'up\ndown\n');
      assert.ok(!stdout.includes('E2E started: linux'));
      console.log(`PASS ${test.name}`);
      continue;
    }
    if (test.failure || test.signal) {
      assert.notEqual(code, 0, test.name);
      assert.deepEqual(after, before, `${test.name}: website changed`);
    } else {
      assert.equal(code, 0, `${test.name}: ${stdout}\n${stderr}`);
      assert.equal(Object.values(after).filter(hash => hash === digest(png)).length, test.count);
      if (test.count === 0) assert.deepEqual(after, before, 'non-export run changed website');
      assert.equal(after['sentinel.txt'], before['sentinel.txt']);
      if (!test.mac) for (const flow of flows) assert.equal(after[`macos/${flow}.png`], before[`macos/${flow}.png`]);
    }
    const events = (await readFile(path.join(cwd, 'events.jsonl'), 'utf8')).trim().split('\n').map(JSON.parse);
    const categories = (await readFile(path.join(cwd, 'categories.jsonl'), 'utf8')).trim().split('\n').map(JSON.parse);
    assert.ok(categories.every(event => event.category === (test.category ?? 'happy-path')), `${test.name}: category not propagated`);
    const starts = events.filter(event => event.state === 'start' && !event.kind.startsWith('prepare'));
    const prefix = starts[0].prefix;
    assert.ok(!prefixes.has(prefix), 'run profile prefix repeated');
    prefixes.add(prefix);
    assert.ok(starts.every(event => event.prefix === prefix));
    const preps = events.filter(event => event.kind.startsWith('prepare'));
    assert.ok(preps.every(event => event.time <= starts[0].time), 'preparation ran during suites');
    if (test.signal) {
      assert.deepEqual(protocolEvents.map(event => event.state), ['all-siblings-ready', test.signal]);
      assert.equal(events.filter(event => event.state === 'ready').length, siblings.length);
      assert.ok(!events.some(event => event.state === 'released'), 'canceled sibling escaped the barrier');
    }
    if (!test.signal) {
      const groups = siblings.map(kind => {
        const group = events.filter(event => event.kind === kind && event.profile !== 'macos');
        const start = group.findLast(event => event.state === 'start');
        return { start: events.indexOf(start), end: events.indexOf(group.at(-1)) };
      });
      assert.ok(Math.max(...groups.map(group => group.start)) < Math.min(...groups.map(group => group.end)), 'groups did not overlap');
      assert.deepEqual(protocolEvents.map(event => event.state), ['all-siblings-ready', 'linux-released', 'linux-completion-reported', 'other-siblings-released']);
      assert.equal(events.filter(event => event.state === 'ready').length, siblings.length);
      for (const kind of ['apple', 'android']) {
        let active = false;
        for (const event of events.filter(event => event.kind === kind)) {
          if (event.state === 'profile-start') { assert.equal(active, false); active = true; }
          if (event.state === 'profile-end') { assert.equal(active, true); active = false; }
        }
      }
      const mac = events.filter(event => event.profile === 'macos');
      assert.equal(mac.length, test.mac ? 2 : 0);
      if (test.mac) assert.ok(starts.filter(event => event.time > starts[0].time).every(event => event.time >= mac.at(-1).time));
      for (const event of starts) {
        assert.equal(event.prepared, '1');
        assert.equal(event.endpoint, event.webEndpoint);
        assert.equal(event.userID, userID, 'actual installation selector not propagated');
        assert.match(event.prefix, /^run-[A-Za-z0-9_-]+$/);
      }
      const artifact = stdout.match(/Persistent Apple E2E artifact path: (.+)/)?.[1];
      if (siblings.includes('apple')) assert.equal(await readFile(path.join(artifact, 'test.log'), 'utf8'), 'retained-diagnostic');
      const diagnostics = stdout.match(/E2E diagnostics: (.+)/)[1];
      for (const name of siblings) {
        const instance = JSON.parse(await readFile(path.join(diagnostics, `${name}-instance.json`), 'utf8'));
        assert.equal(instance.instanceID, 'same-stack');
        const summary = await readFile(path.join(diagnostics, `${name}.summary`), 'utf8');
        assert.match(summary, new RegExp(`^category=${test.category ?? 'happy-path'}\\nexit=\\d+\\nelapsed_seconds=\\d+\\n$`));
        assert.equal(stdout.split(`E2E finished: ${name} `).length, 2, 'completion missing or duplicated');
      }
      if (siblings.includes('apple')) assert.ok(stdout.indexOf('E2E finished: linux ') < stdout.indexOf('E2E finished: apple '), 'fast family reporting waited for Apple');
    }
    for (const pid of new Set(events.map(event => event.pid))) assert.throws(() => process.kill(pid, 0), `child ${pid} survived`);
    if (!test.external) {
      assert.ok(starts.every(event => event.endpoint === 'http://127.0.0.1:50051'));
      assert.equal(await readFile(path.join(cwd, 'backend-events'), 'utf8'), 'up\ndown\n');
    } else {
      assert.ok(starts.every(event => event.endpoint === endpoint));
      assert.equal((await fetch(`${endpoint}/health`)).status, 200, 'caller server stopped');
      assert.equal(await readFile(path.join(cwd, 'backend-events'), 'utf8'), 'wait\n');
    }
    evidence.push({ scenario: test.name, exitCode: code, captures: test.count, childrenStopped: true });
    console.log(`PASS ${test.name}`);
  }
} finally { await new Promise(resolve => caller.close(resolve)); }
await put(path.join(base, 'results.json'), JSON.stringify(evidence, null, 2));
console.log(`Evidence: ${base}`);
