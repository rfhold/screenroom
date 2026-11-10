import { test } from 'node:test';
import assert from 'node:assert/strict';
import { createServer } from 'node:http';
import { once } from 'node:events';
import { spawn } from 'node:child_process';
import { mkdir, readFile, rm } from 'node:fs/promises';
import { fileURLToPath } from 'node:url';
import { probe, runOwnedCommand, stopOwnedGroup } from '../e2e/lifecycle.mjs';
import { verifyIdentity } from '../e2e/backend.mjs';

function trapPostKillRace(t) {
  const original = process.kill.bind(process);
  const kills = new Map();
  t.mock.method(process, 'kill', (pid, signal) => {
    if (pid < 0 && signal === 'SIGKILL') {
      const count = (kills.get(pid) || 0) + 1;
      kills.set(pid, count);
      if (count > 1) throw Object.assign(new Error('Post-termination group race'), { code: 'EPERM' });
    }
    return original(pid, signal);
  });
  return { kills, original };
}

for (const mode of ['headers', 'body']) test(`identity deadline includes stalled ${mode} and cancels the connection`, async () => {
  let closed;
  const disconnected = new Promise(resolve => { closed = resolve; });
  const server = createServer((request, response) => {
    request.on('close', () => closed());
    if (mode === 'body') {
      response.writeHead(200, { 'content-type': 'application/json' });
      response.write('{');
      const trickle = setInterval(() => response.write(' '), 10);
      response.on('close', () => clearInterval(trickle));
    }
  });
  server.listen(0, '127.0.0.1'); await once(server, 'listening');
  const origin = `http://127.0.0.1:${server.address().port}`;
  try {
    const start = Date.now();
    await assert.rejects(verifyIdentity(origin, 'expected', 100), /deadline/);
    assert.ok(Date.now() - start < 1500);
    await Promise.race([disconnected, new Promise((_, reject) => setTimeout(() => reject(new Error('Body connection was not cancelled')), 1000).unref())]);
  } finally { server.closeAllConnections(); await new Promise(resolve => server.close(resolve)); }
});

test('header-only HTTP probe cancels an unfinished body', async () => {
  let closed;
  const disconnected = new Promise(resolve => { closed = resolve; });
  const server = createServer((request, response) => {
    request.on('close', () => closed());
    response.writeHead(200); response.write('unfinished');
  });
  server.listen(0, '127.0.0.1'); await once(server, 'listening');
  try {
    assert.equal((await probe(`http://127.0.0.1:${server.address().port}`, { timeoutMs: 100 })).ok, true);
    await Promise.race([disconnected, new Promise((_, reject) => setTimeout(() => reject(new Error('Body was not cancelled')), 1000).unref())]);
  } finally { server.closeAllConnections(); await new Promise(resolve => server.close(resolve)); }
});

test('attachment deadline avoids post-kill EPERM, stops descendants, and preserves an unrelated server', async t => {
  const { kills, original } = trapPostKillRace(t);
  const directory = fileURLToPath(new URL('../e2e/build/lifecycle/', import.meta.url));
  await mkdir(directory, { recursive: true });
  const heartbeat = `${directory}heartbeat-${process.pid}`;
  const server = createServer((_, response) => response.end('caller-owned'));
  server.listen(0, '127.0.0.1'); await once(server, 'listening');
  const origin = `http://127.0.0.1:${server.address().port}`;
  const descendant = `const fs=require('node:fs');process.on('SIGTERM',()=>{});setInterval(()=>fs.writeFileSync(${JSON.stringify(heartbeat)},String(Date.now())),20);`;
  const parent = `require('node:child_process').spawn(process.execPath,['-e',${JSON.stringify(descendant)}],{stdio:'ignore'});process.on('SIGTERM',()=>{});setInterval(()=>{},1000);`;
  try {
    const start = Date.now();
    await assert.rejects(runOwnedCommand(process.execPath, ['-e', parent], { timeoutMs: 300 }), /deadline/);
    assert.ok(Date.now() - start < 2500);
    assert.equal(kills.size, 1);
    for (const [group, count] of kills) {
      assert.equal(count, 1);
      assert.throws(() => original(-group, 0), { code: 'ESRCH' });
    }
    const stopped = await readFile(heartbeat, 'utf8');
    await new Promise(resolve => setTimeout(resolve, 100));
    assert.equal(await readFile(heartbeat, 'utf8'), stopped);
    assert.equal((await probe(origin)).status, 200);
  } finally { await new Promise(resolve => server.close(resolve)); await rm(heartbeat, { force: true }); }
});

test('owned frontend cleanup avoids post-kill EPERM and accepts an already signalled exit', async t => {
  const { kills } = trapPostKillRace(t);
  const child = spawn(process.execPath, ['-e', "process.on('SIGTERM',()=>{});console.log('ready');setInterval(()=>{},1000)"], { detached: true, stdio: ['ignore', 'pipe', 'inherit'] });
  await once(child.stdout, 'data');
  const start = Date.now();
  await stopOwnedGroup(child, 100);
  assert.ok(Date.now() - start < 1500);
  assert.equal(child.signalCode, 'SIGKILL');
  assert.deepEqual([...kills.values()], [1]);
  await stopOwnedGroup(child, 100);
});

for (const deniedSignal of ['SIGTERM', 'SIGKILL']) test(`live owned process ${deniedSignal} permission denial is rejected, not masked`, async t => {
  const child = spawn(process.execPath, ['-e', "process.on('SIGTERM',()=>{});console.log('ready');setInterval(()=>{},1000)"], { detached: true, stdio: ['ignore', 'pipe', 'inherit'] });
  await once(child.stdout, 'data');
  const original = process.kill.bind(process);
  const denied = Object.assign(new Error('Live owned group permission denied'), { code: 'EPERM' });
  let calls = 0;
  const mocked = t.mock.method(process, 'kill', (pid, signal) => {
    if (pid === -child.pid && signal === deniedSignal) { calls++; throw denied; }
    return original(pid, signal);
  });
  try {
    await assert.rejects(stopOwnedGroup(child, 20), error => error === denied);
    assert.equal(original(child.pid, 0), true);
    await new Promise(resolve => setTimeout(resolve, 50));
    assert.equal(calls, 1);
  } finally { mocked.mock.restore(); await stopOwnedGroup(child, 20); }
});
