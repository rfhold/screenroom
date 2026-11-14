import { defineConfig } from '@playwright/test';
import { selectedCategory } from './e2e/categories.mjs';
import { existsSync, readFileSync } from 'node:fs';
const category = selectedCategory();
const endpoint = process.env.SCREENROOM_E2E_API_ENDPOINT || process.env.SCREENROOM_API_ENDPOINT || 'http://127.0.0.1:50051';
// Custom ports also require the attached backend to allow their browser origin.
const port = process.env.SCREENROOM_E2E_WEB_PORT || '4176';
if (!/^\d+$/.test(port) || Number(port) < 1024 || Number(port) > 65535) throw new Error('SCREENROOM_E2E_WEB_PORT must be an integer from 1024 to 65535');
const origin = `http://127.0.0.1:${Number(port)}`;
const descriptorPath = process.env.SCREENROOM_BACKEND_DESCRIPTOR || new URL(`../../build/backend-runtime/${process.env.SCREENROOM_BACKEND_PROJECT || 'screenroom-local'}/descriptor.json`, import.meta.url);
let userID = process.env.SCREENROOM_USER_ID || '';
if (existsSync(descriptorPath)) {
  const descriptor = JSON.parse(readFileSync(descriptorPath, 'utf8'));
  if (descriptor.ready !== true || !/^[0-9a-f]{8}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{12}$/i.test(descriptor.userID)) throw new Error('Backend descriptor requires the actual installation user selector');
  if (userID && userID !== descriptor.userID) throw new Error('User selector does not match the installation descriptor');
  userID = descriptor.userID;
}

export default defineConfig({
  testDir: './e2e',
  testMatch: 'home.spec.ts',
  outputDir: './e2e/build/results',
  reporter: [['list']],
  grep: category === 'all' ? undefined : new RegExp(`@${category}\\b`),
  fullyParallel: false,
  workers: 1,
  forbidOnly: true,
  timeout: 30_000,
  use: {
    baseURL: origin,
    browserName: 'chromium',
    colorScheme: 'dark',
    locale: 'en-US',
    timezoneId: 'UTC',
    reducedMotion: 'reduce',
    deviceScaleFactor: 1,
    trace: 'retain-on-failure',
    screenshot: 'off'
  },
  projects: [
    { name: 'desktop', use: { viewport: { width: 1440, height: 900 } } },
    { name: 'mobile', use: { viewport: { width: 390, height: 844 }, isMobile: true, hasTouch: true } }
  ],
  globalSetup: './e2e/backend-setup.ts',
  webServer: [{
    command: 'node e2e/dev-server.mjs',
    env: { VITE_SCREENROOM_API_ENDPOINT: endpoint, VITE_SCREENROOM_USER_ID: userID, SCREENROOM_E2E_WEB_PORT: port, SCREENROOM_CORE_PREPARED: '1' },
    url: origin,
    reuseExistingServer: false,
    timeout: 120_000
  }]
});
