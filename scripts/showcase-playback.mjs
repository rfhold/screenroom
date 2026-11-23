import fs from 'node:fs';
import path from 'node:path';
import { pathToFileURL } from 'node:url';
import { inflateSync } from 'node:zlib';

const slugs = new Set(['ios', 'ipados', 'macos', 'tvos', 'android', 'android-tv', 'linux', 'web']);
const signature = Buffer.from('89504e470d0a1a0a', 'hex');
const json = file => JSON.parse(fs.readFileSync(file, 'utf8'));
const save = (file, value) => {
  fs.mkdirSync(path.dirname(file), { recursive: true });
  fs.writeFileSync(`${file}.pending`, JSON.stringify(value, null, 2) + '\n');
  fs.renameSync(`${file}.pending`, file);
};
const requireThat = (ok, message) => { if (!ok) throw new Error(message); };
const exists = file => fs.existsSync(file);
const crcTable = Uint32Array.from({ length: 256 }, (_, value) => {
  for (let bit = 0; bit < 8; bit++) value = (value >>> 1) ^ ((value & 1) ? 0xedb88320 : 0);
  return value >>> 0;
});
export function pngCRC(bytes) {
  let crc = 0xffffffff;
  for (const byte of bytes) crc = crcTable[(crc ^ byte) & 255] ^ (crc >>> 8);
  return (crc ^ 0xffffffff) >>> 0;
}
export function validatePNG(bytes) {
  const valid = (ok, message) => requireThat(ok, `Invalid PNG: ${message}`);
  valid(bytes.length <= 128 * 1024 * 1024 && bytes.subarray(0, 8).equals(signature), 'signature or file size');
  let offset = 8, header, palette, ended = false, dataStarted = false, dataClosed = false, chunks = 0;
  const compressed = [], seen = new Set();
  const beforePalette = new Set(['cHRM', 'gAMA', 'iCCP', 'sBIT', 'sRGB']);
  const beforeData = new Set([...beforePalette, 'bKGD', 'hIST', 'tRNS', 'pHYs', 'sPLT']);
  while (offset < bytes.length) {
    valid(++chunks <= 100000, 'chunk count limit');
    valid(offset + 12 <= bytes.length, 'truncated chunk');
    const length = bytes.readUInt32BE(offset);
    valid(length <= 0x7fffffff && length <= bytes.length - offset - 12, 'chunk bounds');
    const type = bytes.toString('ascii', offset + 4, offset + 8);
    valid(bytes.subarray(offset + 4, offset + 8).every(b => (b >= 65 && b <= 90) || (b >= 97 && b <= 122))
      && /^[A-Za-z]{2}[A-Z][A-Za-z]$/.test(type), 'chunk type');
    const body = bytes.subarray(offset + 8, offset + 8 + length);
    valid(pngCRC(bytes.subarray(offset + 4, offset + 8 + length)) === bytes.readUInt32BE(offset + 8 + length), 'chunk CRC');
    valid(header || type === 'IHDR', 'IHDR must be first');
    valid(!beforeData.has(type) || !dataStarted, 'metadata after IDAT');
    valid(!beforePalette.has(type) || !palette, 'metadata after PLTE');
    if (type !== 'IDAT' && dataStarted) dataClosed = true;
    if (type === 'IHDR') {
      valid(!header && offset === 8 && length === 13, 'IHDR order or length');
      const width = body.readUInt32BE(0), height = body.readUInt32BE(4);
      const depth = body[8], color = body[9], interlace = body[12];
      valid(width > 0 && height > 0 && width <= 32768 && height <= 32768
        && ({ 0: [1, 2, 4, 8, 16], 2: [8, 16], 3: [1, 2, 4, 8], 4: [8, 16], 6: [8, 16] })[color]?.includes(depth)
        && body[10] === 0 && body[11] === 0 && interlace <= 1, 'IHDR fields');
      header = { width, height, depth, color, interlace };
    } else if (type === 'PLTE') {
      valid(!palette && !dataStarted && ![0, 4].includes(header.color)
        && length > 0 && length <= 768 && length % 3 === 0
        && (header.color !== 3 || length / 3 <= 2 ** header.depth), 'PLTE');
      palette = body;
    } else if (type === 'IDAT') {
      valid(!dataClosed && (header.color !== 3 || palette), 'IDAT order or missing PLTE');
      dataStarted = true;
      compressed.push(body);
    } else if (type === 'IEND') {
      valid(length === 0 && dataStarted && offset + 12 === bytes.length, 'IEND or trailing data');
      ended = true;
    } else {
      valid(type[0] === type[0].toLowerCase(), 'unknown critical chunk');
      if (type === 'tRNS') {
        valid(!seen.has(type) && ((header.color === 0 && length === 2)
          || (header.color === 2 && length === 6)
          || (header.color === 3 && palette && length > 0 && length <= palette.length / 3)), 'tRNS');
      }
      if (type === 'hIST') valid(palette && length === palette.length / 3 * 2, 'hIST');
      if (type === 'bKGD') valid(header.color !== 3 || (palette && length === 1 && body[0] < palette.length / 3), 'bKGD');
    }
    seen.add(type);
    offset += length + 12;
  }
  valid(ended, 'missing IEND');
  const channels = ({ 0: 1, 2: 3, 3: 1, 4: 2, 6: 4 })[header.color];
  const passes = header.interlace ? [[0, 0, 8, 8], [4, 0, 8, 8], [0, 4, 4, 8], [2, 0, 4, 4], [0, 2, 2, 4], [1, 0, 2, 2], [0, 1, 1, 2]] : [[0, 0, 1, 1]];
  const rows = passes.map(([x, y, dx, dy]) => {
    const width = Math.max(0, Math.ceil((header.width - x) / dx));
    const height = Math.max(0, Math.ceil((header.height - y) / dy));
    return { height: width ? height : 0, stride: 1 + Math.ceil(width * channels * header.depth / 8) };
  });
  const expected = rows.reduce((sum, row) => sum + row.height * row.stride, 0);
  valid(expected > 0 && expected <= 256 * 1024 * 1024, 'decoded size limit');
  const input = Buffer.concat(compressed);
  const decoded = inflateSync(input, { maxOutputLength: expected, info: true });
  valid(decoded.engine.bytesWritten === input.length && decoded.buffer.length === expected, 'compressed stream or scanline length');
  let rowOffset = 0;
  for (const { height, stride } of rows) {
    for (let y = 0; y < height; y++, rowOffset += stride) valid(decoded.buffer[rowOffset] <= 4, 'scanline filter');
  }
}
function identity(dir) {
  if (!exists(dir)) return null;
  const s = fs.lstatSync(dir);
  requireThat(s.isDirectory() && !s.isSymbolicLink(), 'Unsafe producer directory');
  return `${s.dev}:${s.ino}:${s.birthtimeMs}`;
}
function safePath(root, relative) {
  requireThat(typeof relative === 'string' && /^[A-Za-z0-9_-]+(?:\/[A-Za-z0-9_.-]+)*\.png$/.test(relative)
    && !relative.split('/').some(p => p === '.' || p === '..'), 'Unsafe image path');
  const file = path.join(root, relative);
  let current = root;
  for (const part of relative.split('/')) {
    current = path.join(current, part);
    requireThat(!fs.lstatSync(current).isSymbolicLink(), 'Symlink image path');
  }
  requireThat(fs.realpathSync(file).startsWith(fs.realpathSync(root) + path.sep), 'Image escapes manifest directory');
  return file;
}
function text(value, field) {
  requireThat(typeof value === 'string' && value.trim().length > 0 && value.length <= 2000
    && !/[\u0000-\u001f]|:\/\/|bearer\s+|api[_ -]?key|(?:token|authorization)\s*[:=]|(?:^|[\s"'(])\/[A-Za-z0-9_.-]+\/|[A-Za-z]:\\|XCUIElementType|AccessibilityNodeInfo|<hierarchy|[0-9a-f]{8}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{12}/i.test(value),
  `Unsafe or missing ${field}`);
  for (const selector of [process.env.SCREENROOM_SHOWCASE_ITEM_ID, process.env.SCREENROOM_USER_ID, process.env.SCREENROOM_BACKEND_INSTANCE_ID]) {
    requireThat(!selector || !value.includes(selector), `Private selector in ${field}`);
  }
  return value;
}
function destinationSafe(file) {
  const absolute = path.resolve(file);
  let current = path.parse(absolute).root;
  for (const part of absolute.slice(current.length).split(path.sep)) {
    current = path.join(current, part);
    if (exists(current) || (() => { try { fs.lstatSync(current); return true; } catch { return false; } })()) {
      requireThat(!fs.lstatSync(current).isSymbolicLink(), 'Symlink destination');
    }
  }
}
export function baseline(file, inputs) {
  save(file, {
    started: Date.now(),
    inputs: inputs.map(input => {
      const split = input.lastIndexOf('=');
      requireThat(split > 0, 'Missing expected producer slugs');
      const manifest = path.resolve(input.slice(0, split));
      return { manifest, slugs: input.slice(split + 1).split(','), previous: identity(path.dirname(manifest)) };
    }),
  });
}
export function stage(stageDir, baselineFile) {
  const baseline = json(baselineFile);
  requireThat(process.env.SCREENROOM_BACKEND_INSTANCE_ID && process.env.SCREENROOM_SHOWCASE_ITEM_ID
    && process.env.SCREENROOM_SHOWCASE_TITLE && /^[0-9]{4}$/.test(process.env.SCREENROOM_SHOWCASE_YEAR), 'Missing explicit playback identity');
  const platforms = [], seen = new Set();
  for (const input of baseline.inputs) {
    const dir = path.dirname(input.manifest);
    destinationSafe(input.manifest);
    requireThat(identity(dir) !== input.previous, 'Stale producer directory: producer must recreate playback output');
    requireThat(fs.statSync(dir).birthtimeMs >= baseline.started - 1000, 'Producer directory predates this run');
    const stat = fs.lstatSync(input.manifest);
    requireThat(stat.isFile() && !stat.isSymbolicLink() && stat.mtimeMs >= baseline.started - 1000, 'Stale or unsafe manifest');
    const manifest = json(input.manifest);
    requireThat(manifest.schema_version === 1 && manifest.journey === 'playback', 'Invalid playback schema');
    requireThat(manifest.backend_instance_id === process.env.SCREENROOM_BACKEND_INSTANCE_ID, 'Foreign backend instance');
    requireThat(manifest.item?.id === process.env.SCREENROOM_SHOWCASE_ITEM_ID
      && manifest.item?.title === process.env.SCREENROOM_SHOWCASE_TITLE
      && manifest.item?.year === Number(process.env.SCREENROOM_SHOWCASE_YEAR), 'Foreign item');
    requireThat(Array.isArray(manifest.platforms) && manifest.platforms.length === input.slugs.length, 'Unexpected producer platforms');
    for (const platform of manifest.platforms) {
      const { slug } = platform;
      requireThat(slugs.has(slug) && input.slugs.includes(slug) && !seen.has(slug), 'Unexpected or duplicate platform');
      seen.add(slug);
      requireThat(['observed', 'blocked'].includes(platform.outcome), 'Unknown platform outcome');
      requireThat(Array.isArray(platform.states) && platform.states.length > 0, 'Missing states');
      const ids = new Set(), images = new Set();
      const states = platform.states.map(state => {
        requireThat(typeof state.id === 'string' && /^[a-z0-9]+(?:-[a-z0-9]+)*$/.test(state.id) && !ids.has(state.id), 'Duplicate or unsafe state');
        ids.add(state.id);
        requireThat(['captured', 'unavailable', 'blocked', 'not-observed'].includes(state.status), 'Unknown state status');
        const out = { id: state.id, label: text(state.label, 'label'), status: state.status, viewport: text(state.viewport, 'viewport') };
        if (state.status === 'captured') {
          text(state.image, 'image path');
          const image = safePath(dir, state.image);
          requireThat(!images.has(state.image), 'Duplicate image');
          images.add(state.image);
          const stat = fs.statSync(image);
          requireThat(stat.mtimeMs >= baseline.started - 1000 && stat.size <= 128 * 1024 * 1024, 'Stale or oversized PNG');
          validatePNG(fs.readFileSync(image));
          out.observation = text(state.observation, 'observation');
          out.image = `/showcase/${slug}/playback/${state.image}`;
          const target = path.join(stageDir, 'new', slug, state.image);
          fs.mkdirSync(path.dirname(target), { recursive: true });
          fs.copyFileSync(image, target);
        } else {
          requireThat(state.image === undefined, 'Noncaptured state has image');
          out.reason = text(state.reason, 'reason');
          if (state.observation !== undefined) out.observation = text(state.observation, 'observation');
        }
        return out;
      });
      requireThat(platform.outcome !== 'observed' || !states.some(s => s.status === 'blocked'), 'Blocked states require blocked outcome');
      fs.mkdirSync(path.join(stageDir, 'new', slug), { recursive: true });
      platforms.push({ slug, runtime: text(platform.runtime, 'runtime'), outcome: platform.outcome, states,
        item: { title: text(manifest.item.title, 'title'), year: manifest.item.year },
        provenance: 'Current-run E2E capture · Real Jellyfin-backed application' });
    }
  }
  const metadata = 'website/src/lib/showcase-playback.json';
  destinationSafe(metadata);
  const previous = exists(metadata) ? json(metadata) : { schema_version: 1, platforms: [] };
  requireThat(previous.schema_version === 1 && Array.isArray(previous.platforms)
    && new Set(previous.platforms.map(p => p.slug)).size === previous.platforms.length, 'Invalid existing playback metadata');
  save(path.join(stageDir, 'metadata.json'), { schema_version: 1,
    platforms: [...previous.platforms.filter(p => !seen.has(p.slug)), ...platforms] });
  const targets = [...seen].map(slug => ({ target: `website/static/showcase/${slug}/playback`, source: `new/${slug}` }));
  targets.push({ target: metadata, source: 'metadata.json' });
  for (let i = 0; i < targets.length; i++) {
    const entry = targets[i];
    destinationSafe(entry.target);
    entry.existed = exists(entry.target);
    entry.backup = `original/${i}`;
    if (entry.existed) {
      fs.mkdirSync(path.join(stageDir, 'original'), { recursive: true });
      fs.cpSync(entry.target, path.join(stageDir, entry.backup), { recursive: true, dereference: false });
    }
  }
  save(path.join(stageDir, 'journal.json'), { targets, applied: [] });
}
export function rollback(stageDir) {
  const journalFile = path.join(stageDir, 'journal.json');
  if (!exists(journalFile)) return;
  const journal = json(journalFile);
  for (const i of [...journal.applied].reverse()) {
    const entry = journal.targets[i];
    destinationSafe(entry.target);
    fs.rmSync(entry.target, { recursive: true, force: true });
    if (entry.existed) fs.cpSync(path.join(stageDir, entry.backup), entry.target, { recursive: true });
  }
  journal.applied = [];
  save(journalFile, journal);
}
export function apply(stageDir, afterWrite = () => {}) {
  const journalFile = path.join(stageDir, 'journal.json');
  const journal = json(journalFile);
  try {
    for (let i = 0; i < journal.targets.length; i++) {
      const entry = journal.targets[i];
      destinationSafe(entry.target);
      journal.applied.push(i);
      save(journalFile, journal);
      fs.rmSync(entry.target, { recursive: true, force: true });
      fs.mkdirSync(path.dirname(entry.target), { recursive: true });
      fs.cpSync(path.join(stageDir, entry.source), entry.target, { recursive: true });
      afterWrite(i);
    }
  } catch (error) { rollback(stageDir); throw error; }
}
if (process.argv[1] && import.meta.url === pathToFileURL(path.resolve(process.argv[1])).href) {
  const [command, dir, ...args] = process.argv.slice(2);
  try {
    if (command === 'baseline') baseline(dir, args);
    else if (command === 'stage') stage(dir, args[0]);
    else if (command === 'apply') apply(dir);
    else if (command === 'rollback') rollback(dir);
    else throw new Error('Expected baseline, stage, apply, or rollback');
  } catch (error) { console.error(error.message); process.exitCode = 1; }
}
