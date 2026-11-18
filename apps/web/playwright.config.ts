import { defineConfig } from '@playwright/test'

export default defineConfig({
  testDir: './e2e',
  workers: 1,
  timeout: 90_000,
  expect: { timeout: 15_000 },
  outputDir: '/private/tmp/ferrix-playwright-results',
  use: { baseURL: process.env.FERRIX_E2E_URL, trace: 'retain-on-failure', viewport: { width: 1440, height: 1000 } },
})
