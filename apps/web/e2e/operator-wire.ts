const prefix = '/screenroom.operator.v1.OperatorService/';

function varint(value: number): Buffer {
  const bytes = [];
  while (value >= 128) { bytes.push((value & 127) | 128); value >>>= 7; }
  return Buffer.from([...bytes, value]);
}

function fields(input: Buffer): Map<number, Buffer | number> {
  let offset = 0;
  const read = () => {
    let value = 0;
    for (let shift = 0; shift < 32 && offset < input.length; shift += 7) {
      const byte = input[offset++];
      value += (byte & 127) * 2 ** shift;
      if (!(byte & 128)) return value;
    }
    throw new Error('Invalid fixture varint');
  };
  const result = new Map<number, Buffer | number>();
  while (offset < input.length) {
    const tag = read();
    if ((tag & 7) === 0) result.set(tag >>> 3, read());
    else if ((tag & 7) === 2) {
      const size = read();
      if (offset + size > input.length) throw new Error('Truncated fixture field');
      result.set(tag >>> 3, input.subarray(offset, offset += size));
    } else throw new Error('Unsupported fixture field');
  }
  return result;
}

export function decodeOperator(pathname: string, bytes: Buffer) {
  if (!pathname.startsWith(prefix) || bytes.length < 5 || bytes[0] !== 0 ||
      bytes.readUInt32BE(1) !== bytes.length - 5) throw new Error('Expected unary operator RPC');
  const input = fields(bytes.subarray(5));
  const item = (tag: number) => input.get(tag)?.toString() ?? '';
  const source = `/api/indexing/sources/${item(1)}`;
  const run = `${source}/runs/${item(2)}`;
  const job = `${run}/jobs/${item(4)}`;
  const routes: Record<string, [string, string]> = {
    Health: ['GET', '/api/health'],
    DiscoverConnection: ['POST', '/api/connections/jellyfin/discover'],
    SaveConnection: ['POST', '/api/connections/jellyfin'],
    ListConnections: ['GET', '/api/connections/jellyfin'],
    ListSources: ['GET', '/api/sources'],
    ListLibraries: ['GET', `/api/sources/${item(1)}/libraries`],
    ListRuns: ['GET', `${source}/runs`],
    ListCandidates: ['GET', `${run}/candidates`],
    ListPolicies: ['GET', `${source}/policies`],
    ConfigurePolicy: ['POST', `${source}/policies`],
    GetProgress: ['GET', `${source}/policies/${item(3)}/progress`],
    StartRun: ['POST', `${source}/start`],
    GetRunStatus: ['GET', run],
    GetExecution: ['GET', `${run}/execution`],
    PauseRun: ['POST', `${run}/pause`],
    ResumeRun: ['POST', `${run}/resume`],
    CancelRun: ['POST', `${run}/cancel`],
    ListJobs: ['GET', `${run}/jobs`],
    ListAttempts: ['GET', `${job}/attempts`],
    ListRedrives: ['GET', `${job}/redrives`],
    RedriveJob: ['POST', `${job}/redrive`],
    ObserveQueue: ['POST', '/api/indexing/queue'],
  };
  const route = routes[pathname.slice(prefix.length)];
  if (!route) throw new Error('Unknown operator RPC');
  const [method, path] = route;
  const page = input.get(5);
  return { method, path, body: item(6), after: Buffer.isBuffer(page) ? fields(page).get(1)?.toString() ?? null : null };
}

export function operatorFrame(flag: number, body: Buffer) {
  const header = Buffer.alloc(5);
  header[0] = flag;
  header.writeUInt32BE(body.length, 1);
  return Buffer.concat([header, body]);
}

export function operatorReply(json: string) {
  const payload = Buffer.from(json);
  return Buffer.concat([
    operatorFrame(0, Buffer.concat([Buffer.from([10]), varint(payload.length), payload])),
    operatorFrame(128, Buffer.from('grpc-status: 0\r\n')),
  ]);
}
