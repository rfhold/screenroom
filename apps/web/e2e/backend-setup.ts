import { attachBackend } from './backend.mjs';
export default async function setup() { await attachBackend('web-playwright'); }
