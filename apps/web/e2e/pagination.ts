import { expect, type Page } from '@playwright/test';
import { rpcRoute, type RpcMethod } from './rpc-paths';

function varint(value: number): Buffer {
  const bytes = [];
  do { bytes.push((value & 127) | (value > 127 ? 128 : 0)); value >>>= 7; } while (value);
  return Buffer.from(bytes);
}
function field(value: Buffer, number: number): Buffer {
  return Buffer.concat([varint(number * 8 + 2), varint(value.length), value]);
}
function frame(data: Buffer): Buffer {
  const header = Buffer.alloc(5); header.writeUInt32BE(data.length, 1);
  return Buffer.concat([header, data]);
}
export async function paginate(page: Page, method: RpcMethod, repeat = false) {
  const token = 'opaque:next/page?cursor=two';
  let remaining: Buffer;
  let trailers: Buffer;
  const calls: string[] = [];
  let count = 0;
  await page.route(rpcRoute(method), async route => {
    const continuation = route.request().postDataBuffer()!.includes(Buffer.from(token));
    calls.push(continuation ? token : '');
    if (!continuation) {
      const response = await route.fetch();
      const body = await response.body();
      const end = 5 + body.readUInt32BE(1);
      trailers = body.subarray(end);
      const data = body.subarray(5, end);
      const entries: Buffer[] = [];
      let offset = 0;
      const read = () => { let value = 0, shift = 0, byte; do { byte = data[offset++]; value |= (byte & 127) << shift; shift += 7; } while (byte & 128); return value; };
      while (offset < data.length) {
        const start = offset;
        const tag = read();
        expect(tag & 7).toBe(2);
        const length = read(); offset += length;
        if (tag >>> 3 === 1) entries.push(data.subarray(start, offset));
      }
      count = entries.length;
      expect(count).toBeGreaterThan(1);
      remaining = Buffer.concat(entries);
      await route.fulfill({ response, body: Buffer.concat([frame(Buffer.concat([entries[0], field(Buffer.from(token), 2)])), trailers]) });
    } else {
      // Repeat the first page's last entry to exercise stable ID deduplication.
      const data = repeat ? field(Buffer.from(token), 2) : remaining;
      await route.fulfill({ status: 200, contentType: 'application/grpc-web+proto', headers: { 'access-control-allow-origin': 'http://127.0.0.1:4173' }, body: Buffer.concat([frame(data), trailers]) });
    }
  });
  return { calls, count: () => count, token };
}
