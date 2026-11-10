import { defineConfig } from '@playwright/test';
import { selectedCategory } from './e2e/categories.mjs';
const category = selectedCategory();
const endpoint = process.env.SCREENROOM_E2E_API_ENDPOINT || process.env.SCREENROOM_API_ENDPOINT || 'http://127.0.0.1:50051';

export default defineConfig({
  testDir: './e2e',
  outputDir: './e2e/build/results',
  reporter: [['list']],
  grep: category === 'all' ? undefined : new RegExp(`@${category}\\b`),
  fullyParallel: false,
  workers: 1,
  forbidOnly: true,
  timeout: 30_000,
  use: {
    baseURL: 'http://127.0.0.1:4173',
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
    env: { VITE_SCREENROOM_API_ENDPOINT: endpoint, VITE_SCREENROOM_USER_ID: process.env.SCREENROOM_USER_ID || '00000000-0000-0000-0000-000000000001', SCREENROOM_CORE_PREPARED: '1' },
    url: 'http://127.0.0.1:4173',
    reuseExistingServer: false,
    timeout: 120_000
  }]
});
