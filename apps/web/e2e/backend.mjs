import { readFile } from 'node:fs/promises';
import { fileURLToPath } from 'node:url';
import { probe, runOwnedCommand } from './lifecycle.mjs';

/** @param {string} family */
export async function attachBackend(family) {
  const root = fileURLToPath(new URL('../../../', import.meta.url));
  const endpoint = process.env.SCREENROOM_E2E_API_ENDPOINT || process.env.SCREENROOM_API_ENDPOINT || 'http://127.0.0.1:50051';
  if (process.env.SCREENROOM_API_ENDPOINT && process.env.SCREENROOM_API_ENDPOINT !== endpoint) throw new Error('API endpoints must match');
  await runOwnedCommand('bash', ['scripts/backend.sh', 'wait'], { cwd: root, env: { ...process.env, SCREENROOM_BACKEND_ATTACH: '1' } });
  const descriptor = JSON.parse(await readFile(process.env.SCREENROOM_BACKEND_DESCRIPTOR || `${root}/build/backend-runtime/${process.env.SCREENROOM_BACKEND_PROJECT || 'screenroom-local'}/descriptor.json`, 'utf8'));
  if (!/^[0-9a-f]{8}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{12}$/i.test(descriptor.userID)) throw new Error('Backend descriptor requires the actual installation user selector');
  if (process.env.SCREENROOM_USER_ID && process.env.SCREENROOM_USER_ID !== descriptor.userID) throw new Error('User selector does not match the installation descriptor');
  process.env.SCREENROOM_USER_ID = descriptor.userID;
  if (descriptor.publicOrigin !== endpoint) throw new Error('API origin does not match the caller-owned backend descriptor');
  const { status } = await verifyIdentity(endpoint, descriptor.instanceID);
  const id = descriptor.instanceID;
  const category = family === 'web-production' ? 'production' : process.env.E2E_CATEGORY || 'all';
  for (const entry of category === 'all' ? ['happy-path', 'retry', 'regression'] : [category]) console.log(JSON.stringify({ family, category: entry, endpoint, instance_id: id }));
  return { endpoint, instance_id: id, healthStatus: status, user_id: descriptor.userID };
}

/** @param {string} endpoint @param {string} expectedID @param {number} timeoutMs */
export async function verifyIdentity(endpoint, expectedID, timeoutMs = 5000) {
  const response = await probe(`${endpoint}/__screenroom_instance`, { json: true, timeoutMs });
  if (!response.ok) throw new Error('Backend identity unavailable');
  const actual = response.body;
  const id = actual.instanceID;
  if (!id || id !== expectedID || actual.publicOrigin !== endpoint || (process.env.SCREENROOM_BACKEND_INSTANCE_ID && id !== process.env.SCREENROOM_BACKEND_INSTANCE_ID)) throw new Error('Backend instance mismatch');
  return { status: response.status };
}
