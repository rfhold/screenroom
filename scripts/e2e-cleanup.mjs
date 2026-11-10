import { execFileSync } from 'node:child_process';
import { setTimeout as delay } from 'node:timers/promises';

function processes() {
  const output = execFileSync('ps', ['-axo', 'pid=,ppid=,pgid=,stat=,lstart='], { encoding: 'utf8' });
  const result = new Map();
  for (const line of output.trim().split('\n')) {
    const match = line.match(/^\s*(\d+)\s+(\d+)\s+(\d+)\s+(\S+)\s+(.+)$/);
    if (!match) throw new Error(`Unexpected ps row: ${line}`);
    const [, pid, parent, group, state, started] = match;
    if (!state.startsWith('Z')) result.set(Number(pid), { parent: Number(parent), group: Number(group), started: started.trim() });
  }
  return result;
}

let snapshot = processes();
let groups = new Set();
const anchors = new Map();
for (const identity of process.argv.slice(2)) {
  if (!identity) continue;
  const [pidText, started, kind] = identity.split('|');
  const pid = Number(pidText);
  const current = snapshot.get(pid);
  if (current?.group === pid && current.started === started.trim()) {
    groups.add(current.group);
    if (kind === 'anchor') anchors.set(pid, current.started);
  }
}
const owned = new Map();

function discover(snapshot) {
  let changed = true;
  while (changed) {
    changed = false;
    for (const [pid, info] of snapshot) {
      if (owned.has(pid) || pid === process.pid) continue;
      const parent = snapshot.get(info.parent);
      if (groups.has(info.group) || (parent && owned.get(info.parent) === parent.started)) {
        owned.set(pid, info.started);
        changed = true;
      }
    }
  }
}

function send(snapshot, signal) {
  for (const [pid, started] of owned) {
    if (snapshot.get(pid)?.started !== started) continue;
    try { process.kill(pid, signal); }
    catch (error) { if (error.code !== 'ESRCH') throw error; }
  }
}

function retainGroups(snapshot) {
  groups = new Set([...owned].flatMap(([pid, started]) => {
    const current = snapshot.get(pid);
    if (current?.started !== started) return [];
    const leader = snapshot.get(current.group);
    return groups.has(current.group) || (leader && owned.get(current.group) === leader.started) ? [current.group] : [];
  }));
}

discover(snapshot);
retainGroups(snapshot);
send(snapshot, 'SIGTERM');
const deadline = performance.now() + 25_000;
while (performance.now() < deadline) {
  snapshot = processes();
  // Retain a group only while a known, start-verified member proves its ownership.
  retainGroups(snapshot);
  discover(snapshot);
  // Permit cleanup handlers to finish bounded docker rm or adb reverse commands.
  if (![...owned].some(([pid, started]) => anchors.get(pid) !== started && snapshot.get(pid)?.started === started)) break;
  await delay(200);
}
send(processes(), 'SIGKILL');
