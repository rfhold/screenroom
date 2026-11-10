import { spawn, spawnSync } from 'node:child_process';
const prepared = spawnSync(process.execPath, ['scripts/prepare-core.mjs'], { stdio: 'inherit' });
if (prepared.status !== 0) process.exit(prepared.status ?? 1);
const child = spawn(process.execPath, ['node_modules/vite/bin/vite.js', 'dev', '--host', '127.0.0.1', '--port', '4173', '--strictPort'], { stdio: 'inherit' });
process.on('SIGTERM', () => child.kill('SIGTERM'));
process.on('SIGINT', () => child.kill('SIGINT'));
child.on('exit', code => process.exit(code ?? 1));
