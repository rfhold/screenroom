import { defineConfig } from '@playwright/test';
import { selectedCategory } from './e2e/categories.mjs';
const category = selectedCategory();
const externalEndpoint = process.env.SCREENROOM_E2E_API_ENDPOINT;
const endpoint = externalEndpoint || 'http://127.0.0.1:50063';

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
  webServer: [...(externalEndpoint ? [] : [{ command: 'node e2e/mock-server.mjs', url: `${endpoint}/health`, reuseExistingServer: false, timeout: 120_000 }]), {
    command: 'pnpm dev --host 127.0.0.1 --port 4173 --strictPort',
    env: { VITE_SCREENROOM_API_ENDPOINT: endpoint },
    url: 'http://127.0.0.1:4173',
    reuseExistingServer: false,
    timeout: 120_000
  }]
});
