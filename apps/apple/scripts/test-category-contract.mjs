import assert from 'node:assert/strict';
import { spawnSync } from 'node:child_process';
import { createHash } from 'node:crypto';
import { existsSync, mkdtempSync, readFileSync, readdirSync, rmSync, writeFileSync } from 'node:fs';
import { join } from 'node:path';

const apple = process.argv[2];
const env = { ...process.env };
delete env.E2E_CATEGORY;
const mapping = 'source "$1/scripts/test-category.sh"; configure_apple_test_category; printf "%s\\n" "$apple_test_category" "${apple_test_methods[@]}"';
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
} finally {
    rmSync(directory, { recursive: true });
}
assert.deepEqual(snapshots(), before, 'Invalid category changed capture artifacts');
assert.deepEqual(new Set(readdirSync(runsRoot)), runs, 'Invalid category created a run directory');
console.log(`Category defaults/mapping/negative side-effect checks passed; preserved ${before.size} capture artifacts`);
