import assert from 'node:assert/strict';
import { spawn } from 'node:child_process';
import { createInterface } from 'node:readline';

const url = process.env.DIOXUSCUT_BROWSER_URL ?? 'http://127.0.0.1:1421';
const worker = new URL('../../apps/studio-tauri/scripts/three-render-worker.mjs', import.meta.url);
const moduleUrl = `${url}/src/three-lifecycle-composition.js`;
const child = spawn(process.execPath, [worker.pathname, `--url=${url}`, `--composition-module=${moduleUrl}`], {
  stdio: ['pipe', 'pipe', 'inherit'],
});
const lines = createInterface({ input: child.stdout });
const messages = [];
lines.on('line', (line) => { try { messages.push(JSON.parse(line)); } catch {} });
const waitFor = async (predicate) => {
  while (!messages.some(predicate)) await new Promise((resolve) => setTimeout(resolve, 5));
  return messages.find(predicate);
};

try {
  const ready = await waitFor((message) => message.type === 'ready');
  assert.ok(ready.compositions.includes('three_lifecycle_preview'));
  for (const frame of [0, 15, 30]) {
    child.stdin.write(`${JSON.stringify({ type: 'render', composition: 'three_lifecycle_preview', frame, fps: 30, width: 640, height: 360, props: { color: '#ff8844' } })}\n`);
    const response = await waitFor((message) => message.type === 'frame' && message.frame === frame);
    assert.equal(response.width, 640);
    assert.equal(response.height, 360);
  }
  console.log(JSON.stringify({ composition: 'three_lifecycle_preview', frames: 3, status: 'ok' }));
} finally {
  child.stdin.write('{"type":"shutdown"}\n');
  await new Promise((resolve) => child.once('exit', resolve));
}
