import { spawn } from 'node:child_process';
import { writeFileSync, renameSync } from 'node:fs';

const [statusFile, command, ...args] = process.argv.slice(2);
// Keep the owned process group provable until cleanup reaps every late child.
setInterval(() => {}, 1000);
for (const signal of ['SIGTERM', 'SIGINT', 'SIGHUP']) process.on(signal, () => {});
const child = spawn(command, args, { stdio: 'inherit' });
function complete(status) {
  writeFileSync(`${statusFile}.tmp`, `${status}\n`);
  renameSync(`${statusFile}.tmp`, statusFile);
}
child.on('error', error => { console.error(error); complete(127); });
child.on('exit', (code, signal) => complete(code ?? (signal === 'SIGINT' ? 130 : 143)));
