import { defineConfig } from '@playwright/test';
export default defineConfig({testDir:'.',testMatch:['indexing.spec.ts','connections.spec.ts','public-faults.spec.ts','telemetry.spec.ts'],workers:1,fullyParallel:false,timeout:45000,use:{browserName:'chromium',headless:true,trace:'off',screenshot:'off'},reporter:'list',outputDir:'build/indexing-e2e'});
