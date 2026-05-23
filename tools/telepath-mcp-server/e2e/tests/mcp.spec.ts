import { test, expect } from '@playwright/test';
import { spawn } from 'child_process';
import * as readline from 'readline';
import * as path from 'path';

const SERVER_BIN = path.resolve(__dirname, '../../target/debug/telepath-mcp-server');

// Sends a sequence of JSON-RPC messages to the MCP server and returns a map of
// id → result for all requests that carry an id.  Notifications (no id) are
// written but not awaited.  The process is killed once all expected responses
// have arrived.
async function runMcpSession(
  requests: Array<{ id?: number; method: string; params?: unknown }>,
): Promise<Map<number, unknown>> {
  const proc = spawn(SERVER_BIN, ['--transport', 'loopback'], {
    stdio: ['pipe', 'pipe', 'pipe'],
  });

  const rl = readline.createInterface({ input: proc.stdout! });
  const results = new Map<number, unknown>();
  const pending = new Set(requests.filter(r => r.id !== undefined).map(r => r.id!));

  return new Promise((resolve, reject) => {
    proc.on('error', reject);

    rl.on('line', (line) => {
      let msg: { id?: number; result?: unknown; error?: unknown };
      try { msg = JSON.parse(line); } catch { return; }
      if (msg.id !== undefined) {
        results.set(msg.id, msg.result ?? msg.error);
        pending.delete(msg.id);
        if (pending.size === 0) {
          proc.kill();
          resolve(results);
        }
      }
    });

    for (const req of requests) {
      proc.stdin!.write(JSON.stringify({ jsonrpc: '2.0', ...req }) + '\n');
    }
  });
}

const INIT_SEQUENCE: Array<{ id?: number; method: string; params?: unknown }> = [
  {
    id: 1,
    method: 'initialize',
    params: {
      protocolVersion: '2024-11-05',
      capabilities: {},
      clientInfo: { name: 'test-client', version: '0.0.1' },
    },
  },
  { method: 'notifications/initialized', params: {} },
];

test('ping tool appears in the tool list', async () => {
  const results = await runMcpSession([
    ...INIT_SEQUENCE,
    { id: 2, method: 'tools/list', params: {} },
  ]);

  const list = results.get(2) as { tools: Array<{ name: string }> };
  expect(list.tools.some(t => t.name === 'ping')).toBe(true);
});

test('ping tool returns 0xDEADBEEF', async () => {
  const results = await runMcpSession([
    ...INIT_SEQUENCE,
    { id: 2, method: 'tools/call', params: { name: 'ping', arguments: {} } },
  ]);

  expect(JSON.stringify(results.get(2))).toContain('3735928559');
});
