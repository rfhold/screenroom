import { spawn } from 'node:child_process';
import { fileURLToPath } from 'node:url';
const endpoint = 'http://127.0.0.1:50063';
const child = spawn('bash', ['scripts/mock-server.sh'], {
  cwd: fileURLToPath(new URL('../../../', import.meta.url)),
  stdio: 'inherit',
  env: { ...process.env, SCREENROOM_MOCK_LISTEN: '127.0.0.1:50063', SCREENROOM_MOCK_PUBLIC_BASE: endpoint, SCREENROOM_MOCK_CORS_ORIGINS: 'http://127.0.0.1:4173' }
});
process.on('SIGINT', () => child.kill('SIGINT'));
process.on('SIGTERM', () => child.kill('SIGTERM'));
child.on('exit', code => process.exit(code ?? 0));
