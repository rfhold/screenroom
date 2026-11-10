import { mkdtemp, mkdir, readFile, writeFile, chmod, rm } from 'node:fs/promises';
import { spawnSync } from 'node:child_process';
import path from 'node:path';
import assert from 'node:assert/strict';

await mkdir('build/mock-client-validation', { recursive: true });
const base = await mkdtemp(path.join(process.cwd(), 'build/mock-client-validation/web-cache-'));
async function put(relative, content) {
  const file = path.join(base, relative);
  await mkdir(path.dirname(file), { recursive: true });
  await writeFile(file, content);
}
await put('apps/web/scripts/prepare-core.mjs', await readFile('apps/web/scripts/prepare-core.mjs'));
await put('Cargo.lock', 'wasm-bindgen version fixture');
await put('crates/screenroom-client-core/src/lib.rs', 'fixture core');
await put('scripts/build-client-core.sh', `#!/bin/bash
set -eu
echo build >> builds.txt
mkdir -p target/client-core/wasm
for name in screenroom_client_core.js screenroom_client_core_bg.wasm screenroom_client_core.d.ts screenroom_client_core_bg.wasm.d.ts; do
  echo product > "target/client-core/wasm/$name"
done
`);
for (const name of ['rustc', 'cargo', 'wasm-bindgen']) {
  await put(`bin/${name}`, `#!/bin/bash\necho '${name} fixture' "\${CACHE_TOOLCHAIN:-one}"\n`);
  await chmod(path.join(base, `bin/${name}`), 0o755);
}
const env = { ...process.env, PATH: `${base}/bin:${process.env.PATH}` };
for (const key of ['SCREENROOM_CORE_PREPARED', 'RUSTC', 'WASM_BINDGEN']) delete env[key];
const evidence = [];
async function run(name, expectedBuilds, extra = {}, failure = false) {
  const result = spawnSync(process.execPath, ['apps/web/scripts/prepare-core.mjs'], { cwd: base, env: { ...env, ...extra }, encoding: 'utf8' });
  assert.equal(result.status === 0, !failure, `${name}: ${result.stderr}`);
  const builds = (await readFile(path.join(base, 'builds.txt'), 'utf8')).trim().split('\n').length;
  assert.equal(builds, expectedBuilds, name);
  evidence.push({ scenario: name, builds, exitCode: result.status });
}
await run('cold', 1);
await run('warm', 1);
await run('prepared-warm-verifies', 1, { SCREENROOM_CORE_PREPARED: '1' });
await rm(path.join(base, 'target/client-core/wasm/screenroom_client_core_bg.wasm'));
await run('prepared-missing-fails-without-build', 1, { SCREENROOM_CORE_PREPARED: '1' }, true);
await run('missing-product-rebuilds', 2);
await put('crates/screenroom-client-core/src/lib.rs', 'changed fixture core');
await run('source-change-rebuilds', 3);
await put('target/client-core/wasm/screenroom_client_core.js', 'altered product');
await run('product-content-change-rebuilds', 4);
await run('toolchain-change-rebuilds', 5, { CACHE_TOOLCHAIN: 'two' });
await run('toolchain-warm', 5, { CACHE_TOOLCHAIN: 'two' });
await put('bin/wasm-bindgen', '#!/bin/bash\necho wasm-bindgen-new-version\n');
await run('cli-version-change-rebuilds', 6, { CACHE_TOOLCHAIN: 'two' });
await run('profile-change-rebuilds', 7, { CACHE_TOOLCHAIN: 'two', SCREENROOM_CORE_PROFILE: 'release' });
await put('results.json', JSON.stringify(evidence, null, 2));
console.log(`Passed ${evidence.length} isolated WASM cache scenarios. Evidence: ${base}`);
