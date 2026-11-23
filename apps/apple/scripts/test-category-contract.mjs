import assert from 'node:assert/strict';
import { spawnSync } from 'node:child_process';
import { createHash } from 'node:crypto';
import { existsSync, mkdtempSync, readFileSync, readdirSync, rmSync, writeFileSync } from 'node:fs';
import { join } from 'node:path';

const apple = process.argv[2];
const env = { ...process.env };
delete env.E2E_CATEGORY;
delete env.SCREENROOM_E2E_JOURNEY;
const mapping = 'set -e; source "$1/scripts/test-category.sh"; configure_apple_test_category; printf "%s\\n" "$apple_test_category" "${apple_test_methods[@]}"';
function category(value) {
    const selected = { ...env };
    if (value !== undefined) selected.E2E_CATEGORY = value;
    return spawnSync('/bin/bash', ['-c', mapping, 'category-test', apple], { env: selected, encoding: 'utf8' });
}

const defaultCategory = category();
assert.equal(defaultCategory.status, 0);
assert.equal(defaultCategory.stdout.split('\n')[0], 'all');
const methods = {};
for (const [name, count] of [['happy-path', 1], ['retry', 1], ['regression', 3], ['all', 5]]) {
    const result = category(name);
    assert.equal(result.status, 0);
    methods[name] = result.stdout.trimEnd().split('\n').slice(1);
    assert.equal(methods[name].length, count);
}
assert.deepEqual(new Set(methods.all), new Set([...methods['happy-path'], ...methods.retry, ...methods.regression]));
assert.equal(methods.all.length, new Set(methods.all).size);
const playback = spawnSync('/bin/bash', ['-c', `${mapping}; printf "%s\\n" "$apple_test_class"`, 'playback-test', apple], {
    env: { ...env, SCREENROOM_E2E_JOURNEY: 'playback', E2E_CATEGORY: 'happy-path' }, encoding: 'utf8',
});
assert.equal(playback.status, 0, playback.stderr);
assert.deepEqual(playback.stdout.trimEnd().split('\n'), ['happy-path', 'testNosferatuPlayerStates', 'PlayerScreenshots']);
for (const journey of ['', 'unknown']) {
    const result = spawnSync('/bin/bash', ['-c', mapping, 'journey-test', apple], {
        env: { ...env, SCREENROOM_E2E_JOURNEY: journey }, encoding: 'utf8',
    });
    assert.equal(result.status, 2);
}
for (const category of ['all', 'retry', 'regression']) {
    const result = spawnSync('/bin/bash', [join(apple, 'scripts/e2e.sh'), 'ios'], {
        env: { ...env, SCREENROOM_E2E_JOURNEY: 'playback', E2E_CATEGORY: category }, encoding: 'utf8',
    });
    assert.equal(result.status, 2);
    assert.match(result.stderr, /Playback journey requires/);
}
const captureSource = readFileSync(join(apple, 'E2ETests/PlayerScreenshots.swift'), 'utf8');
assert.ok(captureSource.indexOf('env["SCREENROOM_NATIVE_PLAYBACK_RELEASED"] == "1"') < captureSource.indexOf('XCUIApplication()'));
assert.ok(captureSource.indexOf('env["SCREENROOM_E2E_JOURNEY"] == "playback"') < captureSource.indexOf('XCUIApplication()'));
assert.equal((captureSource.match(/XCTSkip\(/g) ?? []).length, 1, 'Only the pre-application opt-in guard may skip');
assert.ok(!captureSource.includes('Thread.sleep'), 'State evidence must not rely on fixed sleeps');
const runnerSource = readFileSync(join(apple, 'scripts/e2e.sh'), 'utf8');
assert.ok(runnerSource.includes('$apple_test_class/$method'), 'Runner must use the exact journey class');

const runsRoot = join(apple, 'build/e2e/runs');
function snapshots() {
    const hashes = new Map();
    function visit(directory) {
        for (const entry of readdirSync(directory, { withFileTypes: true })) {
            const path = join(directory, entry.name);
            if (entry.isDirectory()) {
                if (entry.name === 'screenshots') {
                    for (const capture of readdirSync(path, { withFileTypes: true })) {
                        if (!capture.isFile()) continue;
                        const file = join(path, capture.name);
                        hashes.set(file, createHash('sha256').update(readFileSync(file)).digest('hex'));
                    }
                } else visit(path);
            }
        }
    }
    visit(runsRoot);
    return hashes;
}

const before = snapshots();
const runs = new Set(readdirSync(runsRoot));
const directory = mkdtempSync(join(apple, 'build/category-contract-'));
const marker = join(directory, 'external-called');
const quote = value => `'${value.replaceAll("'", "'\\''")}'`;
try {
    for (const command of ['xcodegen', 'xcodebuild', 'xcrun', 'cargo', 'rustc', 'rustup', 'curl', 'timeout', 'gtimeout', 'jq']) {
        writeFileSync(join(directory, command), `#!/bin/bash\necho ${command} >> ${quote(marker)}\nexit 97\n`, { mode: 0o755 });
    }
    for (const invalid of ['', 'unknown', 'Happy-Path']) {
        const selected = { ...env, E2E_CATEGORY: invalid, PATH: `${directory}:${env.PATH}` };
        const result = spawnSync('/bin/bash', [join(apple, 'scripts/e2e.sh'), 'ios'], { env: selected, encoding: 'utf8' });
        assert.equal(result.status, 2, result.stderr);
        assert.match(result.stderr, /E2E_CATEGORY must be/);
        assert.equal(existsSync(marker), false, 'Invalid category invoked a protected external command');
    }
    const unreleased = {
        ...env, SCREENROOM_E2E_JOURNEY: 'playback', E2E_CATEGORY: 'happy-path',
        SCREENROOM_API_ENDPOINT: 'http://fixture.invalid:50052', SCREENROOM_BACKEND_DESCRIPTOR: '/fixture/descriptor.json',
        SCREENROOM_BACKEND_INSTANCE_ID: 'fixture-instance',
        SCREENROOM_SHOWCASE_ITEM_ID: 'fixture-item', SCREENROOM_SHOWCASE_TITLE: 'Nosferatu', SCREENROOM_SHOWCASE_YEAR: '1922',
        SCREENROOM_NATIVE_PLAYBACK_RELEASED: '0', PATH: `${directory}:${env.PATH}`,
    };
    const result = spawnSync('/bin/bash', [join(apple, 'scripts/e2e.sh'), 'ios'], { env: unreleased, encoding: 'utf8' });
    assert.equal(result.status, 2, result.stderr);
    assert.match(result.stderr, /SCREENROOM_NATIVE_PLAYBACK_RELEASED=1/);
    assert.equal(existsSync(marker), false, 'Unreleased playback invoked an external command');
    const descriptor = join(directory, 'descriptor.json');
    writeFileSync(descriptor, JSON.stringify({ instanceID: 'fixture-instance', publicOrigin: 'http://127.0.0.1:50052' }));
    const released = {
        ...unreleased, SCREENROOM_NATIVE_PLAYBACK_RELEASED: '1',
        SCREENROOM_BACKEND_ATTACH: '1', SCREENROOM_BACKEND_PROJECT: 'screenroom-jellyfin-local',
        SCREENROOM_API_ENDPOINT: 'http://127.0.0.1:50052', SCREENROOM_BACKEND_INSTANCE_ID: 'fixture-instance',
        SCREENROOM_BACKEND_DESCRIPTOR: descriptor,
    };
    for (const [key, value] of [
        ['SCREENROOM_BACKEND_PROJECT', 'alternate'], ['SCREENROOM_API_ENDPOINT', 'http://127.0.0.1:50051'],
        ['SCREENROOM_BACKEND_ATTACH', undefined], ['SCREENROOM_BACKEND_INSTANCE_ID', undefined],
        ['SCREENROOM_BACKEND_INSTANCE_ID', 'other-instance'],
    ]) {
        const selected = { ...released };
        if (value === undefined) delete selected[key]; else selected[key] = value;
        const rejected = spawnSync('/bin/bash', [join(apple, 'scripts/e2e.sh'), 'ios'], { env: selected, encoding: 'utf8' });
        assert.equal(rejected.status, 2, rejected.stderr);
        assert.equal(existsSync(marker), false, `${key} rejection invoked backend/preparation tooling`);
    }
} finally {
    rmSync(directory, { recursive: true });
}
assert.deepEqual(snapshots(), before, 'Invalid category changed capture artifacts');
assert.deepEqual(new Set(readdirSync(runsRoot)), runs, 'Invalid category created a run directory');
console.log(`Category defaults/mapping/negative side-effect checks passed; preserved ${before.size} capture artifacts`);
