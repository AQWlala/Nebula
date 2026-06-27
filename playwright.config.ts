import { defineConfig } from '@playwright/test';

/**
 * v1.0: Playwright E2E configuration.
 *
 * The tests assume a built bundle (`npm run build`) and the
 * Tauri runtime exposing the dev server on the URL below.
 * In CI we boot the Tauri dev server before running the suite.
 */
export default defineConfig({
  testDir: './e2e',
  timeout: 30_000,
  expect: { timeout: 5_000 },
  fullyParallel: false,
  reporter: [['list'], ['html', { open: 'never' }]],
  use: {
    baseURL: process.env.E2E_BASE_URL || 'http://localhost:1420',
    headless: true,
    viewport: { width: 1280, height: 820 },
    screenshot: 'only-on-failure',
    trace: 'retain-on-failure',
  },
  webServer: {
    command: 'npm run preview -- --port 1420 --strictPort',
    url: 'http://localhost:1420',
    reuseExistingServer: !process.env.CI,
    timeout: 60_000,
  },
  projects: [
    { name: 'chromium', use: { browserName: 'chromium' } },
  ],
});
