import { test, expect, Page } from '@playwright/test';
import { spawn, ChildProcess } from 'child_process';
import * as http from 'http';
import * as path from 'path';

const SERVER_BIN = path.resolve(__dirname, '../../target/debug/telepath-mcp-server');
const INSPECTOR_PORT = 6274;

async function waitForPort(port: number, timeoutMs = 30_000): Promise<void> {
  const deadline = Date.now() + timeoutMs;
  while (Date.now() < deadline) {
    const alive = await new Promise<boolean>((resolve) => {
      const req = http.get(`http://localhost:${port}`, () => {
        req.destroy();
        resolve(true);
      });
      req.on('error', () => resolve(false));
    });
    if (alive) return;
    await new Promise((r) => setTimeout(r, 300));
  }
  throw new Error(`localhost:${port} did not respond within ${timeoutMs}ms`);
}

async function waitForPortClosed(port: number, timeoutMs = 15_000): Promise<void> {
  const deadline = Date.now() + timeoutMs;
  while (Date.now() < deadline) {
    const alive = await new Promise<boolean>((resolve) => {
      const req = http.get(`http://localhost:${port}`, () => { req.destroy(); resolve(true); });
      req.on('error', () => resolve(false));
    });
    if (!alive) return;
    await new Promise((r) => setTimeout(r, 300));
  }
}

let inspectorProc: ChildProcess | undefined;

test.beforeEach(async () => {
  // Start a fresh Inspector for every test so the proxy never carries stale state
  // across tests.  DANGEROUSLY_OMIT_AUTH disables the proxy session-token check so
  // the UI can reach http://localhost:6274 without a token in the URL.
  // detached:true puts Inspector and ALL its child processes (proxy on 6277, client
  // on 6274) into their own process group so afterEach can kill them all at once.
  inspectorProc = spawn('npx', ['@modelcontextprotocol/inspector@latest'], {
    stdio: 'pipe',
    shell: false,
    detached: true,
    env: { ...process.env, DANGEROUSLY_OMIT_AUTH: 'true' },
  });
  inspectorProc.on('error', (err) => {
    throw new Error(`Failed to start Inspector: ${err.message}`);
  });
  await waitForPort(INSPECTOR_PORT);
});

test.afterEach(async () => {
  if (inspectorProc?.pid) {
    try {
      // Kill the entire process group (-pid) so proxy + client sub-processes all die.
      process.kill(-inspectorProc.pid, 'SIGTERM');
    } catch {
      // Process already exited — nothing to do.
    }
    inspectorProc = undefined;
  }
  // Wait until the port is released before the next beforeEach starts a new Inspector.
  await waitForPortClosed(INSPECTOR_PORT);
});

// Connect to the telepath MCP server via the Inspector UI.
// Leaves the browser on the Tools tab with the ping MCP tool visible.
async function connectToTelepath(page: Page): Promise<void> {
  await page.goto(`http://localhost:${INSPECTOR_PORT}`);

  await page.getByRole('textbox', { name: 'Command' }).fill(SERVER_BIN);
  await page
    .getByPlaceholder('Arguments (space-separated)')
    .fill('--transport loopback');

  await page.getByRole('button', { name: 'Connect' }).click();

  // Navigate to the Tools tab to see the MCP tools list.
  // (The Inspector also has a built-in "Ping" tab — we need the Tools tab.)
  await page.getByRole('tab', { name: 'Tools' }).click();

  // In Inspector v0.21.x, tools are not fetched automatically — click "List Tools"
  // to trigger the tools/list RPC call and populate the tools panel.
  await page.getByRole('button', { name: 'List Tools' }).click();

  // Wait until the MCP ping tool appears inside the Tools tabpanel.
  // Using tabpanel scope avoids false-matching the "Ping" tab in the tablist.
  await expect(
    page.getByRole('tabpanel', { name: 'Tools' }).getByText('ping'),
  ).toBeVisible({ timeout: 15_000 });
}

test('ping tool appears in the tool list', async ({ page }) => {
  await connectToTelepath(page);
});

test('ping tool returns 0xDEADBEEF', async ({ page }) => {
  await connectToTelepath(page);

  // Select the ping MCP tool from the Tools panel to open its detail view.
  await page.getByRole('tabpanel', { name: 'Tools' }).getByText('ping').click();

  // Run the tool (Inspector v0.21.x labels the button "Run Tool").
  await page.getByRole('button', { name: 'Run Tool' }).click();

  // The numeric result 3735928559 (= 0xDEADBEEF) should appear in the result area.
  await expect(page.getByText('3735928559')).toBeVisible({ timeout: 10_000 });
});
