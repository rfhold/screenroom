import { spawnSync } from 'node:child_process';
import { fileURLToPath } from 'node:url';
import { createHash } from 'node:crypto';
import { existsSync, readFileSync, readdirSync, writeFileSync } from 'node:fs';
import path from 'node:path';
const root = fileURLToPath(new URL('../../../', import.meta.url));
const output = path.join(root, 'target/client-core/wasm');
const stamp = path.join(output, '.inputs.sha256');
const hash = createHash('sha256');
function files(relative) {
  const full = path.join(root, relative);
  if (!existsSync(full)) return;
  for (const entry of readdirSync(full, { withFileTypes: true }).sort((a, b) => a.name.localeCompare(b.name))) {
    if (entry.name === 'target') continue;
    const name = path.join(relative, entry.name);
    if (entry.isDirectory()) files(name);
    else if (entry.name !== '.inputs.sha256') hash.update(name).update(readFileSync(path.join(root, name)));
  }
}
function command(binary, args) {
  const result = spawnSync(binary, args, { cwd: root, encoding: 'utf8' });
  if (result.status !== 0) throw new Error(`${binary} ${args.join(' ')} failed: ${result.stderr ?? result.error}`);
  hash.update(binary).update(result.stdout);
}
for (const dir of ['crates/screenroom-client-core', 'crates/screenroom-proto', 'proto']) files(dir);
for (const name of ['Cargo.toml', 'Cargo.lock', 'rust-toolchain', 'rust-toolchain.toml', '.cargo/config', '.cargo/config.toml', 'scripts/build-client-core.sh', 'apps/web/scripts/prepare-core.mjs']) {
  if (existsSync(path.join(root, name))) hash.update(name).update(readFileSync(path.join(root, name)));
}
for (const name of ['SCREENROOM_CORE_PROFILE', 'RUSTUP_TOOLCHAIN', 'RUSTC', 'RUSTFLAGS', 'CARGO_ENCODED_RUSTFLAGS', 'CARGO_BUILD_RUSTFLAGS', 'RUSTC_WRAPPER', 'RUSTC_WORKSPACE_WRAPPER', 'WASM_BINDGEN']) hash.update(`${name}=${process.env[name] ?? ''}\n`);
command(process.env.RUSTC || 'rustc', ['-Vv']);
command(process.env.RUSTC || 'rustc', ['--print', 'sysroot']);
command('cargo', ['-Vv']);
const localBindgen = path.join(root, 'target/client-core/tools/bin/wasm-bindgen');
command(process.env.WASM_BINDGEN || (existsSync(localBindgen) ? localBindgen : 'wasm-bindgen'), ['--version']);
const inputs = hash.digest('hex');
function products() {
  const productHash = createHash('sha256');
  if (!existsSync(output)) return '';
  for (const name of readdirSync(output).sort()) {
    if (name === '.inputs.sha256') continue;
    productHash.update(name).update(readFileSync(path.join(output, name)));
  }
  return productHash.digest('hex');
}
const complete = () => ['screenroom_client_core.js', 'screenroom_client_core_bg.wasm', 'screenroom_client_core.d.ts', 'screenroom_client_core_bg.wasm.d.ts'].every(name => existsSync(path.join(output, name)));
const fingerprint = () => `${inputs}\n${products()}\n`;
if (complete() && existsSync(stamp) && readFileSync(stamp, 'utf8') === fingerprint()) process.exit(0);
if (process.env.SCREENROOM_CORE_PREPARED === '1') throw new Error('Prepared WASM core changed; rerun preparation before E2E.');
const result = spawnSync('bash', ['scripts/build-client-core.sh', 'wasm'], { cwd: root, stdio: 'inherit' });
if (result.status !== 0) process.exit(result.status ?? 1);
if (!complete()) throw new Error('WASM build did not produce the complete web package');
writeFileSync(stamp, fingerprint());
