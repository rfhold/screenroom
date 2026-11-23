import fs from 'node:fs';
import path from 'node:path';
import assert from 'node:assert/strict';
import { spawnSync } from 'node:child_process';
import { deflateSync } from 'node:zlib';
import { baseline, stage, apply, rollback, pngCRC, validatePNG } from './showcase-playback.mjs';

const root = process.cwd();
const base = path.join(root, 'build/e2e/tooling-playback');
fs.mkdirSync(base, { recursive: true });
const run = fs.mkdtempSync(path.join(base, 'contract-'));
const pngSignature = Buffer.from('89504e470d0a1a0a', 'hex');
function chunk(type, body = Buffer.alloc(0)) {
  const result = Buffer.alloc(body.length + 12);
  result.writeUInt32BE(body.length);
  result.write(type, 4);
  body.copy(result, 8);
  result.writeUInt32BE(pngCRC(result.subarray(4, -4)), result.length - 4);
  return result;
}
function image({ width = 1, height = 1, depth = 8, color = 6, interlace = 0, raw, compressed, extra = [], omitData = false, omitEnd = false, headerTransform = b => b } = {}) {
  const header = Buffer.alloc(13);
  header.writeUInt32BE(width); header.writeUInt32BE(height, 4);
  header[8] = depth; header[9] = color; header[12] = interlace;
  const channels = ({ 0: 1, 2: 3, 3: 1, 4: 2, 6: 4 })[color] ?? 4;
  const rows = [];
  for (const [x, y, dx, dy] of raw !== undefined || compressed !== undefined ? [] : interlace === 1 ? [[0, 0, 8, 8], [4, 0, 8, 8], [0, 4, 4, 8], [2, 0, 4, 4], [0, 2, 2, 4], [1, 0, 2, 2], [0, 1, 1, 2]] : [[0, 0, 1, 1]]) {
    if (x >= width) continue;
    for (let row = y; row < height; row += dy) {
      const bytes = Buffer.alloc(1 + Math.ceil(Math.ceil((width - x) / dx) * channels * depth / 8));
      bytes[0] = row % 5;
      rows.push(bytes);
    }
  }
  return Buffer.concat([pngSignature, chunk('IHDR', headerTransform(header)),
    ...(color === 3 ? [chunk('PLTE', Buffer.alloc(6))] : []),
    ...extra, ...(!omitData ? [chunk('IDAT', compressed ?? deflateSync(raw ?? Buffer.concat(rows)))] : []),
    ...(!omitEnd ? [chunk('IEND')] : [])]);
}
const png = image();
assert.equal(pngCRC(Buffer.from('IEND')), 0xae426082, 'CRC implementation differs from PNG specification');
const put = (file, bytes) => { fs.mkdirSync(path.dirname(file), { recursive: true }); fs.writeFileSync(file, bytes); };
const writeJSON = (file, value) => put(file, JSON.stringify(value));
const env = {
  SCREENROOM_BACKEND_INSTANCE_ID: 'instance-test',
  SCREENROOM_SHOWCASE_ITEM_ID: 'private-item-selector',
  SCREENROOM_SHOWCASE_TITLE: 'Nosferatu',
  SCREENROOM_SHOWCASE_YEAR: '1922',
};
Object.assign(process.env, env);
function manifest() {
  return { schema_version: 1, journey: 'playback', backend_instance_id: env.SCREENROOM_BACKEND_INSTANCE_ID,
    item: { id: env.SCREENROOM_SHOWCASE_ITEM_ID, title: 'Nosferatu', year: 1922 },
    platforms: [{ slug: 'linux', runtime: 'Mutter test fixture', outcome: 'blocked',
      states: [
        { id: 'error', label: 'Error', status: 'captured', viewport: '1x1', image: 'error.png', observation: 'Visible playback error; fixture, not runtime evidence.' },
        { id: 'playing', label: 'Playing', status: 'blocked', viewport: 'unknown', reason: 'Playback failed.' },
      ] }] };
}
function snapshot(dir = 'website') {
  const result = {};
  function visit(dir) {
    for (const entry of fs.readdirSync(dir, { withFileTypes: true })) {
      const file = path.join(dir, entry.name);
      if (entry.isDirectory()) visit(file);
      else result[file] = fs.readFileSync(file).toString('base64');
    }
  }
  visit(dir);
  return result;
}
let passed = 0;
function fixture(name, mutate, verify) {
  const cwd = path.join(run, name);
  fs.mkdirSync(cwd);
  process.chdir(cwd);
  try {
    put('website/static/showcase/linux/home.png', 'browse');
    put('website/static/showcase/linux/playback/old.png', 'old');
    put('website/static/showcase/macos/playback/old.png', 'unselected');
    writeJSON('website/src/lib/showcase-playback.json', { schema_version: 1, platforms: [{ slug: 'macos', arbitrary: 'unchanged JSON value' }] });
    const before = snapshot();
    baseline('baseline.json', ['producer/playback/manifest.json=linux']);
    put('producer/playback/error.png', png);
    const input = manifest();
    mutate?.(input);
    writeJSON('producer/playback/manifest.json', input);
    verify(input, before);
    passed++;
    console.log(`PASS ${name}`);
  } finally { process.chdir(root); }
}
fixture('sanitized-selected-preservation', null, (_, before) => {
  stage('stage', 'baseline.json');
  assert.deepEqual(snapshot(), before, 'staging changed website');
  apply('stage');
  const metadata = JSON.parse(fs.readFileSync('website/src/lib/showcase-playback.json'));
  assert.deepEqual(metadata.platforms[0], { slug: 'macos', arbitrary: 'unchanged JSON value' });
  assert.equal(metadata.platforms[1].states[0].image, '/showcase/linux/playback/error.png');
  assert.equal(metadata.platforms[1].outcome, 'blocked');
  const serialized = JSON.stringify(metadata);
  for (const value of [env.SCREENROOM_BACKEND_INSTANCE_ID, env.SCREENROOM_SHOWCASE_ITEM_ID]) assert.ok(!serialized.includes(value));
  assert.equal(fs.readFileSync('website/static/showcase/linux/home.png', 'utf8'), 'browse');
  assert.equal(fs.readFileSync('website/static/showcase/macos/playback/old.png', 'utf8'), 'unselected');
  rollback('stage');
  assert.deepEqual(snapshot(), before);
});
const bad = [
  ['foreign-item', m => { m.item.id = 'other'; }],
  ['foreign-title', m => { m.item.title = 'Other'; }],
  ['foreign-year', m => { m.item.year = 1923; }],
  ['foreign-instance', m => { m.backend_instance_id = 'other'; }],
  ['foreign-slug', m => { m.platforms[0].slug = 'web'; }],
  ['unknown-status', m => { m.platforms[0].states[0].status = 'passed'; }],
  ['unknown-outcome', m => { m.platforms[0].outcome = 'passed'; }],
  ['duplicate-platform', m => { m.platforms.push(m.platforms[0]); }],
  ['duplicate-state', m => { m.platforms[0].states.push(m.platforms[0].states[0]); }],
  ['duplicate-image', m => { m.platforms[0].states.push({ ...m.platforms[0].states[0], id: 'another' }); }],
  ['missing-observation', m => { delete m.platforms[0].states[0].observation; }],
  ['missing-reason', m => { delete m.platforms[0].states[1].reason; }],
  ['noncaptured-image', m => { m.platforms[0].states[1].image = 'error.png'; }],
  ['traversal', m => { m.platforms[0].states[0].image = '../error.png'; }],
  ['url', m => { m.platforms[0].states[0].image = 'https://example.com/error.png'; }],
  ['absolute', m => { m.platforms[0].states[0].image = '/error.png'; }],
  ['missing-image', m => { m.platforms[0].states[0].image = 'missing.png'; }],
  ['symlink', () => { fs.renameSync('producer/playback/error.png', 'producer/outside.png'); fs.symlinkSync('../outside.png', 'producer/playback/error.png'); }],
  ['invalid-png', () => put('producer/playback/error.png', 'not png')],
  ['zero-dimensions', () => { const invalid = Buffer.from(png); invalid.writeUInt32BE(0, 16); put('producer/playback/error.png', invalid); }],
  ['truncated-header', () => put('producer/playback/error.png', png.subarray(0, 24))],
  ['header-only-33', () => put('producer/playback/error.png', png.subarray(0, 33))],
  ['truncated-idat', () => put('producer/playback/error.png', png.subarray(0, png.length - 15))],
  ['bad-crc', () => { const invalid = Buffer.from(png); invalid[29] ^= 1; put('producer/playback/error.png', invalid); }],
  ['corrupt-zlib-valid-crc', () => put('producer/playback/error.png', image({ compressed: Buffer.from('not a zlib stream') }))],
  ['short-row', () => put('producer/playback/error.png', image({ raw: Buffer.alloc(4) }))],
  ['long-row', () => put('producer/playback/error.png', image({ raw: Buffer.alloc(6) }))],
  ['illegal-filter', () => put('producer/playback/error.png', image({ raw: Buffer.from([5, 0, 0, 0, 0]) }))],
  ['invalid-ihdr-valid-crc', () => put('producer/playback/error.png', image({ headerTransform: b => { b[10] = 1; return b; } }))],
  ['invalid-depth-valid-crc', () => put('producer/playback/error.png', image({ headerTransform: b => { b[8] = 3; return b; } }))],
  ['invalid-color-valid-crc', () => put('producer/playback/error.png', image({ headerTransform: b => { b[9] = 5; return b; } }))],
  ['invalid-interlace-valid-crc', () => put('producer/playback/error.png', image({ headerTransform: b => { b[12] = 2; return b; } }))],
  ['illegal-adam7-filter', () => { const raw = Buffer.alloc(343); raw[0] = 5; put('producer/playback/error.png', image({ width: 9, height: 9, interlace: 1, raw })); }],
  ['duplicate-ihdr', () => put('producer/playback/error.png', image({ extra: [png.subarray(8, 33)] }))],
  ['missing-palette', () => { const indexed = image({ color: 3 }); put('producer/playback/error.png', Buffer.concat([indexed.subarray(0, 33), indexed.subarray(51)])); }],
  ['nonconsecutive-idat', () => {
    const data = png.subarray(41, png.length - 16);
    put('producer/playback/error.png', Buffer.concat([png.subarray(0, 33), chunk('IDAT', data.subarray(0, 1)), chunk('tEXt', Buffer.from('fixture\0text')), chunk('IDAT', data.subarray(1)), chunk('IEND')]));
  }],
  ['missing-idat', () => put('producer/playback/error.png', image({ omitData: true }))],
  ['missing-iend', () => put('producer/playback/error.png', image({ omitEnd: true }))],
  ['trailing-data', () => put('producer/playback/error.png', Buffer.concat([png, Buffer.from('trailing')]))],
  ['trailing-compressed-data', () => put('producer/playback/error.png', image({ compressed: Buffer.concat([deflateSync(Buffer.alloc(5)), Buffer.from('trailing')]) }))],
  ['huge-decoded-size', () => put('producer/playback/error.png', image({ width: 32768, height: 32768, compressed: deflateSync(Buffer.alloc(5)) }))],
  ['unknown-critical-chunk', () => put('producer/playback/error.png', image({ extra: [chunk('TEST')] }))],
  ['stale-png', () => fs.utimesSync('producer/playback/error.png', 1, 1)],
  ['stale-manifest', () => {
    const old = JSON.parse(fs.readFileSync('baseline.json'));
    const s = fs.statSync('producer/playback');
    old.inputs[0].previous = `${s.dev}:${s.ino}:${s.birthtimeMs}`;
    writeJSON('baseline.json', old);
  }],
  ['private-url', m => { m.platforms[0].states[0].observation = 'http://127.0.0.1/private'; }],
  ['private-selector', m => { m.platforms[0].runtime = env.SCREENROOM_SHOWCASE_ITEM_ID; }],
];
for (const [name, mutate] of bad) fixture(name, mutate, (_, before) => {
  assert.throws(() => stage('stage', 'baseline.json'));
  assert.deepEqual(snapshot(), before);
});
fixture('valid-consecutive-idat', () => {
  const data = png.subarray(41, png.length - 16);
  put('producer/playback/error.png', Buffer.concat([png.subarray(0, 33), chunk('IDAT', data.subarray(0, 1)), chunk('IDAT', data.subarray(1)), chunk('IEND')]));
}, (_, before) => {
  stage('stage', 'baseline.json');
  assert.deepEqual(snapshot(), before);
});
for (const [color, depths] of [[0, [1, 2, 4, 8, 16]], [2, [8, 16]], [3, [1, 2, 4, 8]], [4, [8, 16]], [6, [8, 16]]]) {
  for (const depth of depths) for (const interlace of [0, 1]) {
    fixture(`valid-color-${color}-depth-${depth}-interlace-${interlace}`, () => put('producer/playback/error.png', image({ width: 9, height: 9, color, depth, interlace })), (_, before) => {
      validatePNG(fs.readFileSync('producer/playback/error.png'));
      stage('stage', 'baseline.json');
      assert.deepEqual(snapshot(), before);
    });
  }
}
for (const at of [0, 1]) fixture(`rollback-after-write-${at}`, null, (_, before) => {
  stage('stage', 'baseline.json');
  assert.throws(() => apply('stage', i => { if (i === at) throw new Error('injected copy failure'); }));
  assert.deepEqual(snapshot(), before);
});
for (const at of [0, 1]) fixture(`interrupted-export-journal-${at}`, null, (_, before) => {
  stage('stage', 'baseline.json');
  const code = `import {apply} from ${JSON.stringify('file://' + path.join(root, 'scripts/showcase-playback.mjs'))};
    apply('stage', i => { if(i === ${at}) process.kill(process.pid, 'SIGKILL'); });`;
  const child = spawnSync(process.execPath, ['--input-type=module', '-e', code]);
  assert.equal(child.signal, 'SIGKILL');
  rollback('stage');
  assert.deepEqual(snapshot(), before);
});
for (const scenario of ['all', 'env', 'failed-family', 'interrupted-export', 'missing-selector', 'no-attach', 'false-attach', 'alternate-project', 'endpoint-50051', 'endpoint-alias-only', 'wrong-category']) {
  const cwd = path.join(run, `root-${scenario}`);
  fs.mkdirSync(cwd);
  for (const file of ['e2e.sh', 'e2e-group.mjs', 'e2e-cleanup.mjs', 'showcase-playback.mjs']) {
    put(path.join(cwd, 'scripts', file), fs.readFileSync(path.join(root, 'scripts', file)));
  }
  const project = 'screenroom-jellyfin-local';
  const user = '12345678-1234-1234-1234-123456789abc';
  const instance = '87654321-1234-1234-1234-123456789abc';
  put(path.join(cwd, 'scripts/backend.sh'), `#!/bin/bash
set -eu
echo "$1" >> backend-events
[ "$1" = wait ] || exit 99
mkdir -p build/backend-runtime/${project}
echo '{"ready":true,"userID":"${user}","instanceID":"${instance}"}' > build/backend-runtime/${project}/descriptor.json
`);
  const suite = `
import fs from 'node:fs';
import path from 'node:path';
const [kind, ...profiles] = process.argv.slice(2);
const event = state => fs.appendFileSync('events.jsonl', JSON.stringify({kind,state,time:Date.now(),journey:process.env.SCREENROOM_E2E_JOURNEY,item:process.env.SCREENROOM_SHOWCASE_ITEM_ID,user:process.env.SCREENROOM_USER_ID,instance:process.env.SCREENROOM_BACKEND_INSTANCE_ID})+'\\n');
event('start');
if (kind === 'prepare') { event('end'); process.exit(); }
if (kind === 'linux' && process.env.FAIL_FAMILY === '1') process.exit(17);
let groups;
if (kind === 'apple') {
  if (process.env.SCREENROOM_NATIVE_PLAYBACK_RELEASED !== '1') throw new Error('Root did not release sequential native playback ownership');
  fs.mkdirSync(process.env.E2E_APPLE_RUN_DIR);
  groups = profiles.map(profile => ({dir:process.env.E2E_APPLE_RUN_DIR+'/'+profile+'/playback', slugs:[profile==='ipad'?'ipados':profile]}));
} else {
  groups = [{ dir:kind === 'web'?'apps/web/e2e/build/screenshots/playback':'apps/'+kind+'/build/e2e/playback', slugs:kind==='android'?['android','android-tv']:[kind] }];
}
for (const {dir,slugs} of groups) {
  fs.rmSync(dir,{recursive:true,force:true}); fs.mkdirSync(dir,{recursive:true});
  const platforms = slugs.map(slug => {
    fs.writeFileSync(dir+'/'+slug+'.png',Buffer.from('${png.toString('base64')}','base64'));
    return {slug,runtime:'Isolated orchestration fixture',outcome:'observed',states:[{id:'playing',label:'Playing fixture',status:'captured',viewport:'1x1',image:slug+'.png',observation:'Isolated fixture only; not playback evidence.'}]};
  });
  fs.writeFileSync(dir+'/manifest.json',JSON.stringify({schema_version:1,journey:'playback',backend_instance_id:process.env.SCREENROOM_BACKEND_INSTANCE_ID,item:{id:process.env.SCREENROOM_SHOWCASE_ITEM_ID,title:process.env.SCREENROOM_SHOWCASE_TITLE,year:Number(process.env.SCREENROOM_SHOWCASE_YEAR)},platforms}));
}
event('end');
`;
  put(path.join(cwd, 'suite.mjs'), suite);
  for (const [file, body] of Object.entries({
    'apps/apple/scripts/prepare-core.sh': 'node suite.mjs prepare',
    'apps/android/scripts/build-core.sh': 'node suite.mjs prepare',
    'apps/apple/scripts/e2e.sh': 'node suite.mjs apple ${1:-ios ipad tvos}',
    'apps/android/scripts/e2e.sh': 'node suite.mjs android',
    'apps/linux/tests/e2e/run.sh': 'node suite.mjs linux',
    'bin/pnpm': 'if [ "${1-}" = --version ]; then echo 12.18.0; else node suite.mjs web; fi',
    'bin/docker': 'exit 99',
  })) {
    put(path.join(cwd, file), `#!/bin/bash\nset -eu\n${body}\n`);
    fs.chmodSync(path.join(cwd, file), 0o755);
  }
  put(path.join(cwd, 'apps/web/scripts/prepare-core.mjs'), "process.argv.push('prepare'); await import('../../../suite.mjs');");
  if (scenario === 'interrupted-export') {
    put(path.join(cwd, 'bin/node'), `#!/bin/bash
set -eu
if [ "\${1-}" = scripts/showcase-playback.mjs ] && [ "\${2-}" = apply ]; then
  exec "${process.execPath}" --input-type=module -e 'import {apply} from "./scripts/showcase-playback.mjs"; apply(process.argv[1], i => { if(i===7) process.kill(process.ppid,"SIGTERM"); });' "$3"
fi
exec "${process.execPath}" "$@"
`);
    fs.chmodSync(path.join(cwd, 'bin/node'), 0o755);
  }
  put(path.join(cwd, 'website/static/showcase/macos/home.png'), 'historical browse');
  writeJSON(path.join(cwd, 'website/src/lib/showcase-playback.json'), { schema_version: 1, platforms: [] });
  const childEnv = { ...process.env, ...env, PATH: `${cwd}/bin:${process.env.PATH}`,
    E2E_MACOS: scenario === 'all' ? '1' : '0', SCREENROOM_BACKEND_ATTACH: '1',
    SCREENROOM_BACKEND_PROJECT: project, SCREENROOM_API_ENDPOINT: 'http://127.0.0.1:50052' };
  for (const key of ['E2E_CATEGORY', 'SCREENROOM_E2E_API_ENDPOINT', 'SCREENROOM_USER_ID', 'SCREENROOM_BACKEND_DESCRIPTOR', 'E2E_APPLE_RUN_DIR', 'SCREENROOM_NATIVE_PLAYBACK_RELEASED']) delete childEnv[key];
  childEnv.SCREENROOM_E2E_JOURNEY = scenario === 'env' ? 'playback' : 'invalid-overridden-by-cli';
  if (scenario === 'missing-selector') delete childEnv.SCREENROOM_SHOWCASE_ITEM_ID;
  if (scenario === 'no-attach') delete childEnv.SCREENROOM_BACKEND_ATTACH;
  if (scenario === 'false-attach') childEnv.SCREENROOM_BACKEND_ATTACH = '0';
  if (scenario === 'alternate-project') childEnv.SCREENROOM_BACKEND_PROJECT = 'poison-caller-project';
  if (scenario === 'endpoint-50051') childEnv.SCREENROOM_API_ENDPOINT = 'http://127.0.0.1:50051';
  if (scenario === 'endpoint-alias-only') {
    delete childEnv.SCREENROOM_API_ENDPOINT;
    childEnv.SCREENROOM_E2E_API_ENDPOINT = 'http://127.0.0.1:50052';
  }
  if (scenario === 'failed-family') childEnv.FAIL_FAMILY = '1';
  const args = ['scripts/e2e.sh', '--update-showcase', ...(scenario === 'env' ? [] : ['--journey', 'playback']),
    ...(scenario === 'wrong-category' ? ['--category', 'retry'] : [])];
  const child = spawnSync('bash', args, { cwd, env: childEnv, encoding: 'utf8', timeout: 30000 });
  put(path.join(cwd, 'result.log'), child.stdout + child.stderr);
  if (['missing-selector', 'no-attach', 'false-attach', 'alternate-project', 'endpoint-50051', 'endpoint-alias-only', 'wrong-category'].includes(scenario)) {
    assert.equal(child.status, 2, child.stdout + child.stderr);
    assert.ok(!fs.existsSync(path.join(cwd, 'build')), 'early rejection created side effects');
    assert.ok(!fs.existsSync(path.join(cwd, 'backend-events')), 'early rejection accessed caller backend');
    assert.ok(!fs.existsSync(path.join(cwd, 'events.jsonl')), 'early rejection launched preparation or suite');
    assert.deepEqual(JSON.parse(fs.readFileSync(path.join(cwd, 'website/src/lib/showcase-playback.json'))), { schema_version: 1, platforms: [] }, 'early rejection exported metadata');
  } else {
    assert.equal(fs.readFileSync(path.join(cwd, 'backend-events'), 'utf8'), 'wait\n', 'playback changed backend lifecycle');
    const events = fs.readFileSync(path.join(cwd, 'events.jsonl'), 'utf8').trim().split('\n').map(JSON.parse).filter(e => e.kind !== 'prepare');
    let active = false;
    for (const event of events) {
      if (event.state === 'start') { assert.equal(active, false, 'families overlapped'); active = true; }
      else { assert.equal(active, true); active = false; }
      assert.equal(event.journey, 'playback');
      assert.equal(event.item, env.SCREENROOM_SHOWCASE_ITEM_ID);
      assert.equal(event.instance, instance);
      assert.equal(event.user, user);
    }
    const metadata = JSON.parse(fs.readFileSync(path.join(cwd, 'website/src/lib/showcase-playback.json')));
    if (scenario === 'failed-family' || scenario === 'interrupted-export') {
      assert.notEqual(child.status, 0);
      assert.deepEqual(metadata.platforms, []);
      if (scenario === 'failed-family') assert.ok(!events.some(e => e.kind === 'web'), 'family launched after failure');
      else assert.ok(!fs.existsSync(path.join(cwd, 'website/static/showcase/linux/playback')), 'interrupted images not rolled back');
    } else {
      assert.equal(child.status, 0, child.stdout + child.stderr);
      assert.deepEqual(new Set(metadata.platforms.map(p => p.slug)), new Set(['ios', 'ipados', 'tvos', 'android', 'android-tv', 'linux', 'web', ...(scenario === 'all' ? ['macos'] : [])]));
      if (scenario === 'all') {
        assert.equal(events[0].kind, 'apple');
        assert.ok(events[1].state === 'end' && events[2].kind === 'apple', 'Mac did not finish before Apple batch');
      }
    }
  }
  assert.equal(fs.readFileSync(path.join(cwd, 'website/static/showcase/macos/home.png'), 'utf8'), 'historical browse');
  passed++;
  console.log(`PASS root-${scenario}`);
}
console.log(`${passed} playback contract tests passed. Evidence: ${run}`);
