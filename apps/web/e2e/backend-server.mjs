import { attachBackend } from './backend.mjs';
import { readFile } from 'node:fs/promises';
import { fileURLToPath } from 'node:url';

const root = fileURLToPath(new URL('../../../', import.meta.url));
process.env.SCREENROOM_BACKEND_DESCRIPTOR ||= `${root}/build/backend-runtime/${process.env.SCREENROOM_BACKEND_PROJECT || 'screenroom-local'}/descriptor.json`;
await attachBackend('web-e2e-preflight');
const descriptor = JSON.parse(await readFile(process.env.SCREENROOM_BACKEND_DESCRIPTOR, 'utf8'));
if (descriptor.ready !== true || !/^[0-9a-f]{8}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{12}$/i.test(descriptor.userID)) {
  throw new Error('Backend descriptor requires the actual installation user selector');
}
if (process.env.SCREENROOM_USER_ID && process.env.SCREENROOM_USER_ID !== descriptor.userID) {
  throw new Error('User selector does not match the installation descriptor');
}
process.env.SCREENROOM_USER_ID = descriptor.userID;
