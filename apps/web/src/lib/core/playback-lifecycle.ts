const players = new Set<() => void>();
export function registerPlayback(stop: () => void) {
  players.add(stop);
  return () => { players.delete(stop); };
}
export function revokePlayback() {
  for (const stop of [...players]) stop();
}
