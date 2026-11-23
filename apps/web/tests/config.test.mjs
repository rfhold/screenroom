import { test } from 'node:test';
import assert from 'node:assert/strict';
import { readFile } from 'node:fs/promises';
import { stripTypeScriptTypes } from 'node:module';

const source = stripTypeScriptTypes(await readFile(new URL('../src/lib/core/config.ts', import.meta.url), 'utf8'));
const browser = { location: { origin: 'https://web.example.test' } };
const explicit = {
  VITE_SCREENROOM_API_ENDPOINT: 'http://127.0.0.1:50051',
  VITE_SCREENROOM_USER_ID: 'build-selector'
};

async function config(env, window) {
  const module = `import.meta.env = ${JSON.stringify(env)};\nconst window = ${JSON.stringify(window)};\n${source}`;
  const { webConfig } = await import(`data:text/javascript;base64,${Buffer.from(module).toString('base64')}`);
  return webConfig();
}

test('standalone Web defaults to the preview API and an empty selector', async () => {
  const expected = { endpoint: 'https://preview-screenroom.holdenitdown.net', user_id: '' };
  assert.deepEqual(await config({}, browser), expected);
  assert.deepEqual(await config({ VITE_SCREENROOM_API_ENDPOINT: '' }, browser), expected);
  assert.deepEqual(await config({}), expected);
});

test('standalone Web uses the explicit build endpoint and selector', async () => {
  assert.deepEqual(await config(explicit, browser), {
    endpoint: explicit.VITE_SCREENROOM_API_ENDPOINT,
    user_id: explicit.VITE_SCREENROOM_USER_ID
  });
});

test('same-origin mode uses the browser origin instead of the build endpoint', async () => {
  assert.deepEqual(await config({ ...explicit, VITE_SCREENROOM_SAME_ORIGIN: '1' }, browser), {
    endpoint: browser.location.origin,
    user_id: ''
  });
});

test('an injected selector uses the browser origin and takes precedence over build settings', async () => {
  for (const selector of ['runtime-selector', '']) {
    assert.deepEqual(await config(explicit, { ...browser, __SCREENROOM_USER_ID__: selector }), {
      endpoint: browser.location.origin,
      user_id: selector
    });
  }
});
