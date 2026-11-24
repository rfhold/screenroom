import { mkdtemp, mkdir, readFile, writeFile, chmod } from 'node:fs/promises';
import { spawnSync } from 'node:child_process';
import path from 'node:path';
import assert from 'node:assert/strict';

await mkdir('build/mock-client-validation', { recursive: true });
const base = await mkdtemp(path.join(process.cwd(), 'build/mock-client-validation/apple-cache-'));
async function put(name, contents) {
  const file = path.join(base, name);
  await mkdir(path.dirname(file), { recursive: true });
  await writeFile(file, contents);
}
await put('apps/apple/scripts/prepare-core.sh', await readFile('apps/apple/scripts/prepare-core.sh'));
for (const dir of ['crates/screenroom-client-core', 'crates/screenroom-proto', 'crates/screenroom-observability', 'proto']) await mkdir(path.join(base, dir), { recursive: true });
for (const file of ['Cargo.toml', 'Cargo.lock', 'scripts/patch-swift-cancellation.py']) await put(file, 'fixture');
await put('scripts/build-client-core.sh', `#!/bin/bash
set -eu
echo build >> builds
mkdir -p target/client-core/apple/ScreenroomClientCore.xcframework target/client-core/bindings
echo framework > target/client-core/apple/ScreenroomClientCore.xcframework/Info.plist
for file in ScreenroomClientCore.swift ScreenroomClientCoreFFI.h ScreenroomClientCoreFFI.modulemap; do echo binding > "target/client-core/bindings/$file"; done
`);
for (const name of ['rustup', 'rustc', 'selected-rustc', 'other-rustc', 'cargo', 'xcode-select', 'xcodebuild', 'xcrun', 'wrapper', 'workspace-wrapper']) {
  await put(`bin/${name}`, `#!/bin/bash\necho '${name} fixture' "$*"\n`);
  await chmod(path.join(base, `bin/${name}`), 0o755);
}
const env = { ...process.env, PATH: `${base}/bin:${process.env.PATH}` };
for (const name of ['SCREENROOM_CORE_PREPARED', 'RUSTC', 'RUSTC_WRAPPER', 'RUSTC_WORKSPACE_WRAPPER', 'CARGO_BUILD_RUSTC', 'CARGO_BUILD_RUSTC_WRAPPER', 'CARGO_BUILD_RUSTC_WORKSPACE_WRAPPER', 'RUSTFLAGS', 'CARGO_ENCODED_RUSTFLAGS', 'CARGO_BUILD_RUSTFLAGS']) delete env[name];
let builds = 0;
const evidence = [];
async function run(name, prepared, failure = false) {
  const result = spawnSync('bash', ['apps/apple/scripts/prepare-core.sh'], { cwd: base, env: { ...env, SCREENROOM_CORE_PREPARED: prepared ? '1' : '0' }, encoding: 'utf8' });
  assert.equal(result.status === 0, !failure, `${name}: ${result.stderr}`);
  const actual = (await readFile(path.join(base, 'builds'), 'utf8')).trim().split('\n').length;
  assert.equal(actual, builds, `${name}: unexpected build count`);
  evidence.push({ scenario: name, exitCode: result.status, builds });
}
async function changed(name) {
  await run(`${name}-prepared-rejects`, true, true);
  builds++;
  await run(`${name}-normal-rebuilds`, false);
  await run(`${name}-prepared-warm`, true);
}
builds++;
await run('cold', false);
await run('warm', false);
await run('prepared-warm', true);
env.RUSTC = path.join(base, 'bin/selected-rustc');
await changed('RUSTC-selection');
env.RUSTC = path.join(base, 'bin/other-rustc');
await changed('RUSTC-path');
await put('bin/other-rustc', '#!/bin/bash\necho changed-compiler "$*"\n');
await changed('RUSTC-content-version-sysroot');
delete env.RUSTC;
env.CARGO_BUILD_RUSTC = path.join(base, 'bin/selected-rustc');
await changed('CARGO_BUILD_RUSTC-selection');
for (const name of ['RUSTC_WRAPPER', 'RUSTC_WORKSPACE_WRAPPER', 'CARGO_BUILD_RUSTC_WRAPPER', 'CARGO_BUILD_RUSTC_WORKSPACE_WRAPPER']) {
  const relative = `bin/${name}`;
  await put(relative, '#!/bin/bash\necho wrapper\n');
  await chmod(path.join(base, relative), 0o755);
  env[name] = path.join(base, relative);
  await changed(`${name}-selection`);
  await put(relative, '#!/bin/bash\necho altered-wrapper\n');
  await changed(`${name}-content`);
}
for (const name of ['RUSTFLAGS', 'CARGO_ENCODED_RUSTFLAGS', 'CARGO_BUILD_RUSTFLAGS', 'CARGO_TARGET_AARCH64_APPLE_IOS_SIM_RUSTFLAGS']) {
  env[name] = '-Copt-level=1';
  await changed(name);
}
await put('.cargo/config.toml', '[build]\nrustflags = ["-Copt-level=2"]\n');
await changed('Cargo-config');
await put('scripts/patch-swift-cancellation.py', 'changed patcher fixture');
await changed('Swift-patcher');
await put('target/client-core/bindings/ScreenroomClientCore.swift', 'altered generated binding');
await changed('Swift-product');
await put('results.json', JSON.stringify(evidence, null, 2));
console.log(`Passed ${evidence.length} isolated Apple cache checks. Evidence: ${base}`);
