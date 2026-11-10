import { test } from 'node:test';
import assert from 'node:assert/strict';
import { createHash } from 'node:crypto';
import { spawnSync } from 'node:child_process';
import { fileURLToPath } from 'node:url';
import { readFile, readdir, stat, writeFile, utimes } from 'node:fs/promises';

const web = new URL('../', import.meta.url);
const build = new URL('../e2e/build/', import.meta.url);
async function tree(directory) {
  const entries = [];
  for (const name of (await readdir(directory)).sort()) {
    const url = new URL(name, directory);
    const metadata = await stat(url);
    entries.push([url.href, metadata.mtimeMs, metadata.size, metadata.isFile() ? createHash('sha256').update(await readFile(url)).digest('hex') : await tree(new URL(`${name}/`, directory))]);
  }
  return entries;
}

test('CLI no-match discovery preserves ten sentinel captures and all test artifacts', async () => {
  const originals = [];
  try {
    for (const flow of ['home', 'sidebar', 'library', 'search', 'detail']) {
      for (const profile of ['desktop', 'mobile']) {
        const url = new URL(`../e2e/build/screenshots/${flow}-${profile}.png`, import.meta.url);
        originals.push({ url, bytes: await readFile(url), metadata: await stat(url) });
        await writeFile(url, `sentinel:${flow}:${profile}`);
      }
    }
    const before = await tree(build);
    for (const args of [
      ['--grep', '__no_matching_test__'],
      ['--project', 'mobile', '--grep', '__no_matching_test__'],
      ['--grep', 'pagination'],
      ['--grep=/pagination/i'],
      ['-gpagination'],
      ['__no_matching_path__.spec.ts']
    ]) {
      const result = spawnSync(process.execPath, [fileURLToPath(new URL('../e2e/run.mjs', import.meta.url)), ...args], { cwd: web, env: { ...process.env, E2E_CATEGORY: 'happy-path' }, encoding: 'utf8' });
      assert.equal(result.status, 1);
      assert.match(result.stdout + result.stderr, /No tests found/);
      assert.doesNotMatch(result.stdout + result.stderr, /WebServer|prepare-core|vite dev|Screenroom mock/);
      assert.deepEqual(await tree(build), before);
    }
    for (const category of ['', 'unknown']) {
      const result = spawnSync(process.execPath, [fileURLToPath(new URL('../e2e/run.mjs', import.meta.url))], { cwd: web, env: { ...process.env, E2E_CATEGORY: category }, encoding: 'utf8' });
      assert.equal(result.status, 1);
      assert.match(result.stderr, /Invalid E2E_CATEGORY/);
      assert.deepEqual(await tree(build), before);
    }
  } finally {
    for (const { url, bytes, metadata } of originals) {
      await writeFile(url, bytes);
      await utimes(url, metadata.atime, metadata.mtime);
    }
  }
});
