import { defineConfig } from '@playwright/test';

export default defineConfig({ testDir: '.', testMatch: 'maintenance.spec.ts', workers: 1, fullyParallel: false, timeout: 45000, use: { browserName: 'chromium', headless: true }, reporter: 'list', outputDir: 'build/maintenance-e2e' });
