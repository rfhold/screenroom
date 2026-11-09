import { spawnSync } from 'node:child_process';
import { createRequire } from 'node:module';
import { selectedCategory } from './categories.mjs';

const category = selectedCategory();
const cli = createRequire(import.meta.url).resolve('@playwright/test/cli');
const args = process.argv.slice(2);
if (category !== 'all') {
  let grep;
  for (let index = 0; index < args.length; index++) {
    if (args[index] === '--grep' || args[index] === '-g') grep = args[++index];
    else if (args[index].startsWith('--grep=')) grep = args[index].slice('--grep='.length);
    else if (args[index].startsWith('-g') && args[index].length > 2) grep = args[index].slice(2);
  }
  const literal = grep?.match(/^\/(.*)\/([a-z]*)$/s);
  const source = literal ? literal[1] : grep;
  const flags = literal ? literal[2] : '';
  const pattern = `(?=[\\s\\S]*@${category}\\b)${source === undefined ? '' : `(?=[\\s\\S]*(?:${source}))`}`;
  args.push('--grep', `/${pattern}/${flags}`);
}
const discoveryEnv = { ...process.env };
for (const key of ['PLAYWRIGHT_JSON_OUTPUT_FILE', 'PLAYWRIGHT_JSON_OUTPUT_DIR', 'PLAYWRIGHT_JSON_OUTPUT_NAME']) delete discoveryEnv[key];
const discovery = spawnSync(process.execPath, [cli, 'test', ...args, '--list', '--reporter=json'], { encoding: 'utf8', env: discoveryEnv });
if (discovery.status !== 0) {
  process.stderr.write(discovery.stderr || discovery.stdout || String(discovery.error));
  process.exit(discovery.status || 1);
}
const report = JSON.parse(discovery.stdout);
/** @typedef {{specs?: {tests: unknown[]}[], suites?: Suite[]}} Suite */
/** @param {Suite[]} suites @returns {number} */
function countTests(suites = []) {
  return suites.reduce((count, suite) => count + (suite.specs ?? []).reduce((sum, spec) => sum + spec.tests.length, 0) + countTests(suite.suites), 0);
}
if (!countTests(report.suites)) {
  console.error('No tests found. No artifacts or runtime preparation changed.');
  process.exit(1);
}
await import('./clean-screenshots.mjs');
const result = spawnSync(process.execPath, [cli, 'test', ...args], { stdio: 'inherit' });
if (result.error) console.error(result.error);
process.exit(result.status ?? 1);
