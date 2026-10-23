import { defineConfig, devices } from '@playwright/test'

export default defineConfig({
  testDir: './e2e',
  fullyParallel: false,
  workers: 1,
  timeout: 30_000,
  use: {
    ...devices['Desktop Chrome'],
    baseURL: process.env.WEB_BASE_URL,
    trace: 'off', // Account passwords/link tokens must not enter durable test artifacts.
    screenshot: 'off',
    video: 'off',
  },
})
