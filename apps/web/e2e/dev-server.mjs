import { spawn, spawnSync } from 'node:child_process';
const port = process.env.SCREENROOM_E2E_WEB_PORT || '4176';
if (!/^\d+$/.test(port) || Number(port) < 1024 || Number(port) > 65535) throw new Error('SCREENROOM_E2E_WEB_PORT must be an integer from 1024 to 65535');
const prepared = spawnSync(process.execPath, ['scripts/prepare-core.mjs'], { stdio: 'inherit' });
if (prepared.status !== 0) process.exit(prepared.status ?? 1);
const child = spawn(process.execPath, ['node_modules/vite/bin/vite.js', 'dev', '--host', '127.0.0.1', '--port', String(Number(port)), '--strictPort'], { stdio: 'inherit' });
process.on('SIGTERM', () => child.kill('SIGTERM'));
process.on('SIGINT', () => child.kill('SIGINT'));
child.on('exit', code => process.exit(code ?? 1));
