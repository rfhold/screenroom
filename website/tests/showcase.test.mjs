import assert from 'node:assert/strict';
import { readFile, writeFile, unlink } from 'node:fs/promises';
import { randomUUID } from 'node:crypto';
import test from 'node:test';
import ts from 'typescript';
import { compile } from 'svelte/compiler';
import { render } from 'svelte/server';

const source = await readFile(new URL('../src/lib/showcase.ts', import.meta.url), 'utf8');
const javascript = ts.transpileModule(
  source.replace("import playback from './showcase-playback.json';", 'const playback = { schema_version: 1, platforms: [] };'),
  { compilerOptions: { target: ts.ScriptTarget.ES2022, module: ts.ModuleKind.ES2022 } }
).outputText;
const { shells, captures, playbackPlatforms } = await import(`data:text/javascript;base64,${Buffer.from(javascript).toString('base64')}`);

const platform = {
  slug: 'web', runtime: 'Test browser', outcome: 'blocked',
  item: { title: 'Nosferatu', year: 1922 },
  provenance: 'Current-run E2E capture · Real Jellyfin-backed application',
  states: [
    { id: 'desktop-error', label: 'Playback error', status: 'captured', viewport: '1280 × 720',
      image: '/showcase/web/playback/error.png', observation: 'The player displays a media error.' },
    ...['unavailable', 'blocked', 'not-observed'].map((status) => ({
      id: status, label: status, status, viewport: '1280 × 720',
      reason: `Reason for ${status}`
    }))
  ]
};

test('historical browse paths and Home thumbnails remain unchanged across all shells', () => {
  assert.equal(shells.length, 8);
  const paths = [];
  for (const shell of shells) {
    assert.equal(shell.flows[0].slug, 'home');
    assert.deepEqual(shell.flows.slice(0, 5).map((flow) => flow.slug), ['home', 'sidebar', 'library', 'search', 'detail']);
    for (const flow of shell.flows.slice(0, 5)) {
      assert.match(flow.provenance, /Historical/);
      for (const screenshot of flow.screenshots) {
        const expected = shell.slug === 'web'
          ? `/showcase/web/${screenshot.viewport}${flow.slug === 'home' ? '' : `-${flow.slug}`}.png`
          : `/showcase/${shell.slug}/${flow.slug}.png`;
        assert.equal(screenshot.src, expected);
        paths.push(screenshot.src);
      }
    }
    assert.match(shell.flows[5].caption, /No playback screenshots for this app/);
    assert.deepEqual(shell.flows[5].screenshots, []);
  }
  assert.equal(new Set(paths).size, 45);
});

test('metadata validates state statuses and local platform PNG paths', () => {
  const valid = { schema_version: 1, platforms: [platform] };
  assert.deepEqual(playbackPlatforms(valid), [platform]);
  for (const state of playbackPlatforms(valid)[0].states.slice(1)) {
    assert.ok(!Object.hasOwn(state, 'observation'));
  }
  for (const image of ['https://example.com/a.png', '//example.com/a.png', '/showcase/ios/playback/a.png',
    '/showcase/web/playback/../a.png', '/showcase/web/playback/a.png?token=secret', '/showcase/web/playback/a.svg']) {
    const invalid = structuredClone(valid);
    invalid.platforms[0].states[0].image = image;
    assert.throws(() => playbackPlatforms(invalid));
  }
  for (const mutate of [
    (value) => { value.platforms[0].states[0].status = 'success'; },
    (value) => { delete value.platforms[0].states[0].observation; },
    (value) => { value.platforms[0].states[1].observation = ''; },
    (value) => { delete value.platforms[0].states[1].reason; },
    (value) => { value.platforms[0].states[1].image = '/showcase/web/playback/a.png'; },
    (value) => { value.platforms.push(value.platforms[0]); },
    (value) => { value.platforms[0].states.push(value.platforms[0].states[0]); },
    (value) => { value.platforms[0].runtime = ''; }
  ]) {
    const invalid = structuredClone(valid);
    mutate(invalid);
    assert.throws(() => playbackPlatforms(invalid));
  }
});

test('shell renderer distinguishes error evidence, noncaptured states, and absent platforms', async () => {
  let page = await readFile(new URL('../src/routes/showcase/[shell]/+page.svelte', import.meta.url), 'utf8');
  page = page.replace(/<script lang="ts">([\s\S]*?)<\/script>/, (_, script) =>
    `<script>${ts.transpileModule(script.replace(/import .*?;\n/g, ''),
      { compilerOptions: { target: ts.ScriptTarget.ES2022, module: ts.ModuleKind.ES2022 } }).outputText.replace('export {};', '')}
    const shells = [];</script>`);
  const filename = new URL(`./.showcase-${randomUUID()}.mjs`, import.meta.url);
  try {
    await writeFile(filename, compile(page, { generate: 'server' }).js.code);
    const { default: Page } = await import(filename.href);
    const shell = { slug: 'web', name: 'Web', description: 'Test shell', flows: captures('web', 'Web', [platform]) };
    const { body } = render(Page, { props: { data: { shell } } });
    assert.match(body, /Nosferatu \(1922\)/);
    assert.match(body, /Platform outcome: blocked/);
    assert.match(body, /This run did not demonstrate successful playback/);
    assert.match(body, /Runtime: Test browser/);
    assert.match(body, /Playback error/);
    assert.match(body, /href="\/showcase\/web\/playback\/error.png"/);
    assert.equal((body.match(/src="\/showcase\/web\/playback\//g) ?? []).length, 1);
    for (const status of ['unavailable', 'blocked', 'not-observed']) assert.ok(body.includes(`Reason for ${status}`));
    assert.doesNotMatch(body, /undefined/);
    const capturesForReview = ['desktop-preparing', 'desktop-audio-tracks', 'mobile-playing', 'desktop-playing'].map((id) => ({
      ...platform.states[0], id, label: id, image: `/showcase/web/playback/${id}.png`
    }));
    shell.flows = captures('web', 'Web', [{ ...platform, outcome: 'observed', states: capturesForReview }]);
    const review = render(Page, { props: { data: { shell } } }).body;
    assert.deepEqual(
      [...review.matchAll(/src="\/showcase\/web\/playback\/([^"]+)\.png"/g)].map(([, id]) => id),
      ['desktop-playing', 'mobile-playing', 'desktop-audio-tracks']
    );
    assert.match(review, /<details class="capture-details[^"]*"><summary\b[^>]*>All playback captures and test notes<\/summary>/);
    assert.match(review, /href="\/showcase\/web\/playback\/desktop-preparing\.png"/);
    assert.doesNotMatch(review, /This run did not demonstrate successful playback/);
    shell.flows = captures('web', 'Web', []);
    const missing = render(Page, { props: { data: { shell } } }).body;
    assert.match(missing, /No playback screenshots for this app/);
    assert.doesNotMatch(missing, /src="[^"]*\/playback\//);
  } finally {
    await unlink(filename).catch(() => {});
  }
});
