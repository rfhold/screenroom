import { spawn } from 'node:child_process';

/** @param {import('node:child_process').ChildProcess} child @param {NodeJS.Signals} signal */
function signalOwnedGroup(child, signal) {
  if (!child.pid) return;
  try { process.kill(-child.pid, signal); }
  catch (error) { if (/** @type {NodeJS.ErrnoException} */ (error).code !== 'ESRCH') throw error; }
}

/** Only accepts a child created with detached: true.
 * @param {import('node:child_process').ChildProcess} child @param {number} graceMs */
export async function stopOwnedGroup(child, graceMs = 1000) {
  if (child.exitCode !== null || child.signalCode !== null) return;
  await new Promise((resolve, reject) => {
    let escalated = false;
    let settled = false;
    /** @param {Error | undefined} error */
    function finish(error) {
      if (settled) return;
      settled = true;
      clearTimeout(escalation); clearTimeout(deadline);
      child.removeListener('exit', exited);
      if (error) reject(error); else resolve(undefined);
    }
    function forceKill() {
      try { signalOwnedGroup(child, 'SIGKILL'); escalated = true; }
      catch (error) { finish(/** @type {Error} */ (error)); }
    }
    function exited() { if (!escalated) forceKill(); finish(undefined); }
    const escalation = setTimeout(forceKill, graceMs);
    const deadline = setTimeout(() => finish(new Error('Owned process did not exit after termination')), graceMs + 1000);
    child.once('exit', exited);
    try { signalOwnedGroup(child, 'SIGTERM'); }
    catch (error) { finish(/** @type {Error} */ (error)); }
  });
}

/** @param {string} url @param {{ timeoutMs?: number, json?: boolean }} options */
export async function probe(url, { timeoutMs = 5000, json = false } = {}) {
  const controller = new AbortController();
  const timer = setTimeout(() => controller.abort(new Error(`Probe deadline exceeded: ${url}`)), timeoutMs);
  try {
    const response = await fetch(url, { signal: controller.signal });
    const body = json && response.ok ? await response.json() : undefined;
    return { ok: response.ok, status: response.status, body };
  } finally {
    clearTimeout(timer);
    controller.abort();
  }
}

/** @param {string} command @param {string[]} args @param {{ cwd?: string, env?: NodeJS.ProcessEnv, timeoutMs?: number }} options */
export async function runOwnedCommand(command, args, { cwd, env = process.env, timeoutMs = 120_000 } = {}) {
  await new Promise((resolve, reject) => {
    const child = spawn(command, args, { cwd, env, stdio: 'inherit', detached: true });
    let expired = false;
    let escalated = false;
    let settled = false;
    /** @type {ReturnType<typeof setTimeout> | undefined} */
    let escalation;
    /** @type {ReturnType<typeof setTimeout> | undefined} */
    let finalDeadline;
    /** @param {Error | undefined} error */
    function finish(error) {
      if (settled) return;
      settled = true;
      clearTimeout(timer); clearTimeout(escalation); clearTimeout(finalDeadline);
      child.removeListener('exit', exited);
      if (error) reject(error); else resolve(undefined);
    }
    const timer = setTimeout(() => {
      expired = true;
      try { signalOwnedGroup(child, 'SIGTERM'); }
      catch (error) { finish(/** @type {Error} */ (error)); return; }
      escalation = setTimeout(() => {
        try { signalOwnedGroup(child, 'SIGKILL'); escalated = true; }
        catch (error) { finish(/** @type {Error} */ (error)); }
      }, 500);
      finalDeadline = setTimeout(() => finish(new Error('Attachment subprocess deadline exceeded')), 1500);
    }, timeoutMs);
    child.once('error', finish);
    /** @param {number | null} code */
    function exited(code) {
      try { if (expired && !escalated) signalOwnedGroup(child, 'SIGKILL'); }
      catch (error) { finish(/** @type {Error} */ (error)); return; }
      finish(expired ? new Error('Attachment subprocess deadline exceeded') : code === 0 ? undefined : new Error(`Attachment subprocess exited: ${code}`));
    }
    child.once('exit', exited);
  });
}
