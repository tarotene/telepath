import { defineConfig, devices } from '@playwright/test';

export default defineConfig({
  testDir: './tests',
  timeout: 60_000,
  workers: 1,
  reporter: [['list'], ['html', { open: 'never' }]],
  projects: [
    // Layer 1: Headless MCP protocol tests — fast, no browser required
    {
      name: 'mcp-headless',
      testMatch: /mcp\.spec\.ts/,
    },
    // Layer 2: Inspector UI tests — uncomment when re-enabling browser tests
    // Requires: npx playwright install chromium
    // {
    //   name: 'inspector-ui',
    //   testMatch: /inspector\.spec\.ts/,
    //   use: { ...devices['Desktop Chrome'], headless: true },
    // },
  ],
});
