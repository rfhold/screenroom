import type { Page } from '@playwright/test';

type Field = { number: number; wire: number; value: Buffer };
const varint = (value: number) => {
  const bytes: number[] = [];
  do { bytes.push((value & 127) | (value > 127 ? 128 : 0)); value >>>= 7; } while (value);
  return Buffer.from(bytes);
};
function fields(data: Buffer): Field[] {
  let offset = 0;
  const read = () => { let value = 0, shift = 0, byte; do { byte = data[offset++]; value |= (byte & 127) << shift; shift += 7; } while (byte & 128); return value; };
  const result: Field[] = [];
  while (offset < data.length) {
    const tag = read(), wire = tag & 7;
    const start = offset;
    const length = wire === 2 ? read() : wire === 1 ? 8 : wire === 5 ? 4 : 0;
    if (wire === 0) { read(); result.push({ number: tag >>> 3, wire, value: data.subarray(start, offset) }); }
    else { result.push({ number: tag >>> 3, wire, value: data.subarray(offset, offset + length) }); offset += length; }
  }
  return result;
}
const encode = (entries: Field[]) => Buffer.concat(entries.map(field => Buffer.concat([varint(field.number * 8 + field.wire), ...(field.wire === 2 ? [varint(field.value.length)] : []), field.value])));
const string = (number: number, value: string): Field => ({ number, wire: 2, value: Buffer.from(value) });
const number = (field: number, value: number): Field => { const bytes = Buffer.alloc(8); bytes.writeDoubleLE(value); return { number: field, wire: 1, value: bytes }; };
const nested = (entries: Field[], number: number, update: (fields: Field[]) => Field[]) => entries.map(field => field.number === number ? { ...field, value: encode(update(fields(field.value))) } : field);

export async function serverVariant(page: Page) {
  let notice = 'Alternate server notice';
  await page.route('**/screenroom.client.v1.**', async route => {
    const method = route.request().url().split('/').at(-1);
    if (!['GetCatalogInfo', 'ListLibraries', 'GetSearchCapabilities', 'GetItem'].includes(method!)) return route.continue();
    const response = await route.fetch(), body = await response.body();
    const end = 5 + body.readUInt32BE(1);
    let entries = fields(body.subarray(5, end));
    if (method === 'ListLibraries') entries = nested(entries, 1, entry => entry.map(field => field.number === 2 && field.value.toString() === 'Movies' ? string(2, 'Cinema from server') : field));
    if (method === 'GetSearchCapabilities') entries = [string(1, 'text'), string(2, 'Movie'), string(3, 'all'), string(4, 'Server capability description'), { number: 5, wire: 0, value: varint(37) }];
    if (method === 'GetCatalogInfo') entries = nested(entries, 1, info => nested(info, 2, entry => entry.map(field => field.number === 4 ? string(4, notice) : field)));
    if (method === 'GetItem') entries = nested(entries, 1, entry => [
      ...entry.filter(field => ![15, 17, 19, 20].includes(field.number)),
      { number: 15, wire: 2, value: encode([string(1, 'Archive'), number(2, 3.5), number(3, 5), string(5, '2030-01-02')]) },
      { number: 17, wire: 2, value: encode([string(1, 'Archive'), string(2, 'Verified'), string(3, 'https://example.invalid/catalog')]) },
      string(19, 'Server-specific provenance'), string(20, '2030-01-03'),
    ]);
    const data = encode(entries), header = Buffer.alloc(5); header.writeUInt32BE(data.length, 1);
    await route.fulfill({ response, body: Buffer.concat([header, data, body.subarray(end)]) });
  });
  return { updateNotice(value: string) { notice = value; } };
}

export function requestLibrary(body: Buffer, fieldNumber: number) {
  return fields(body.subarray(5, 5 + body.readUInt32BE(1))).find(field => field.number === fieldNumber)?.value.toString();
}

export async function duplicateLibraryLabels(page: Page) {
  await page.route('**/screenroom.client.v1.BrowseService/ListLibraries', async route => {
    const response = await route.fetch(), body = await response.body();
    const end = 5 + body.readUInt32BE(1);
    const entries = nested(fields(body.subarray(5, end)), 1, entry => {
      const id = entry.find(field => field.number === 1)?.value.toString();
      return ['movies', 'tv'].includes(id!) ? entry.map(field => field.number === 2 ? string(2, 'Shared library') : field) : entry;
    });
    const data = encode(entries), header = Buffer.alloc(5); header.writeUInt32BE(data.length, 1);
    await route.fulfill({ response, body: Buffer.concat([header, data, body.subarray(end)]) });
  });
}

export async function refreshedDetail(page: Page) {
  await page.route('**/screenroom.client.v1.CatalogService/*', async route => {
    const method = route.request().url().split('/').at(-1);
    if (!['GetItem', 'ListCredits', 'GetCatalogInfo'].includes(method!)) return route.continue();
    const response = await route.fetch(), body = await response.body();
    const end = 5 + body.readUInt32BE(1);
    let entries = fields(body.subarray(5, end));
    if (method === 'GetItem') entries = nested(entries, 1, entry => [...entry.filter(field => field.number !== 19), string(19, 'Fresh detail metadata from RPC')]);
    if (method === 'ListCredits') entries = nested(entries, 1, entry => entry.map(field => field.number === 3 ? string(3, `Fresh crew role from RPC: ${field.value.toString()}`) : field));
    if (method === 'GetCatalogInfo') entries = nested(entries, 1, info => nested(info, 2, entry => entry.map(field => field.number === 4 ? string(4, 'Fresh catalog notice from RPC') : field)));
    const data = encode(entries), header = Buffer.alloc(5); header.writeUInt32BE(data.length, 1);
    await route.fulfill({ response, body: Buffer.concat([header, data, body.subarray(end)]) });
  });
}
