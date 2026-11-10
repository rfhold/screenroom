import { spawn, execFileSync } from 'node:child_process';
import { mkdir, mkdtemp, writeFile, readFile } from 'node:fs/promises';
import { setTimeout as delay } from 'node:timers/promises';
import { once } from 'node:events';
import path from 'node:path';
import assert from 'node:assert/strict';

await mkdir('build/mock-client-validation', { recursive: true });
const base = await mkdtemp(path.join(process.cwd(), 'build/mock-client-validation/cleanup-regression-'));
const handler = path.join(base, 'handler.mjs');
await writeFile(handler, `import { spawn } from 'node:child_process';
import { writeFileSync } from 'node:fs';
process.on('SIGTERM', () => setTimeout(() => {
  const child = spawn('sleep', ['60'], { stdio: 'ignore' });
  writeFileSync(${JSON.stringify(path.join(base, 'late.pid'))}, String(child.pid));
  process.exit(0);
}, 60));
writeFileSync(${JSON.stringify(path.join(base, 'ready'))}, String(process.pid));
setInterval(() => {}, 1000);
`);
const guardian = spawn(process.execPath, ['scripts/e2e-group.mjs', path.join(base, 'group.status'), process.execPath, handler], { detached: true, stdio: 'ignore' });
const guardianExit = once(guardian, 'exit');
const caller = spawn('sleep', ['60'], { stdio: 'ignore' });
const callerExit = once(caller, 'exit');
function active(pid) {
  try { return !execFileSync('ps', ['-p', String(pid), '-o', 'stat='], { encoding: 'utf8' }).trim().startsWith('Z'); }
  catch { return false; }
}
let late;
try {
  for (let attempt = 0; ; attempt++) {
    try { await readFile(path.join(base, 'ready')); break; }
    catch { assert.ok(attempt < 100, 'handler readiness'); await delay(50); }
  }
  const started = execFileSync('ps', ['-p', String(guardian.pid), '-o', 'lstart='], { encoding: 'utf8' }).trim();
  const mismatch = spawn(process.execPath, ['scripts/e2e-cleanup.mjs', `${guardian.pid}|wrong-start|anchor`], { stdio: 'inherit' });
  assert.equal((await once(mismatch, 'exit'))[0], 0);
  assert.ok(active(guardian.pid), 'mismatched PID identity killed guardian');
  const callerStarted = execFileSync('ps', ['-p', String(caller.pid), '-o', 'lstart='], { encoding: 'utf8' }).trim();
  const before = performance.now();
  const cleanup = spawn(process.execPath, ['scripts/e2e-cleanup.mjs', `${guardian.pid}|${started}|anchor`, `${caller.pid}|${callerStarted}`], { stdio: 'inherit' });
  assert.equal((await once(cleanup, 'exit'))[0], 0);
  const elapsed = performance.now() - before;
  assert.ok(elapsed < 30_000, 'cleanup exceeded bounded grace');
  late = Number(await readFile(path.join(base, 'late.pid'), 'utf8'));
  assert.equal((await readFile(path.join(base, 'group.status'), 'utf8')).trim(), '0', 'TERM handler did not spawn and exit');
  await guardianExit;
  assert.equal(active(late), false, 'reparented late child survived');
  assert.ok(active(caller.pid), 'caller-owned sentinel stopped');
  await writeFile(path.join(base, 'results.json'), JSON.stringify({ lateChildStopped: true, handlerExited: true, guardianStopped: true, mismatchedIdentityPreserved: true, callerPreserved: true, elapsedMs: elapsed }, null, 2));
  console.log(`PASS TERM-spawn-and-exit, bounded escalation, PID identity, and caller ownership. Evidence: ${base}`);
} finally {
  if (active(guardian.pid)) process.kill(-guardian.pid, 'SIGKILL');
  if (late && active(late)) process.kill(late, 'SIGKILL');
  caller.kill('SIGKILL');
  await Promise.all([guardianExit, callerExit]);
}
