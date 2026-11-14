import { defineConfig } from '@playwright/test';

export default defineConfig({
  testDir: '.', testMatch: 'incremental.spec.ts', workers: 1, timeout: 60_000,
  outputDir: './build/incremental-results',
  globalSetup: './backend-setup.ts',
  use: { baseURL: process.env.SCREENROOM_E2E_WEB_URL || 'http://127.0.0.1:4175', viewport: { width: 1440, height: 900 } },
  webServer: process.env.SCREENROOM_E2E_WEB_URL ? undefined : {
    command: 'node node_modules/vite/bin/vite.js dev --host 127.0.0.1 --port 4175 --strictPort',
    cwd: new URL('..', import.meta.url).pathname,
    env: { VITE_SCREENROOM_API_ENDPOINT: process.env.SCREENROOM_E2E_API_ENDPOINT || process.env.SCREENROOM_API_ENDPOINT || 'http://127.0.0.1:50051', VITE_SCREENROOM_USER_ID: process.env.SCREENROOM_USER_ID || '', SCREENROOM_CORE_PREPARED: '1' },
    url: 'http://127.0.0.1:4175', reuseExistingServer: false,
  },
});
