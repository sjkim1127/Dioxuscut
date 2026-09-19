import assert from 'node:assert/strict';
import { spawn } from 'node:child_process';
import { readFileSync, unlinkSync } from 'node:fs';
import { createInterface } from 'node:readline';

const url = process.env.DIOXUSCUT_BROWSER_URL ?? 'http://127.0.0.1:1421';
const worker = new URL('../../apps/studio-tauri/scripts/three-render-worker.mjs', import.meta.url);
const moduleUrl = `${url}/src/three-lifecycle-composition.js`;
const child = spawn(process.execPath, [worker.pathname, `--url=${url}`, `--composition-module=${moduleUrl}`], {
  stdio: ['pipe', 'pipe', 'inherit'],
});
const lines = createInterface({ input: child.stdout });
const messages = [];
lines.on('line', (line) => {
  try {
    messages.push(JSON.parse(line));
  } catch (err) {
    console.error('NON_JSON LINE:', line);
  }
});
const waitFor = async (predicate, timeoutMs = 15000) => {
  const started = Date.now();
  while (true) {
    const index = messages.findIndex(predicate);
    if (index >= 0) return messages.splice(index, 1)[0];
    if (Date.now() - started > timeoutMs) {
      throw new Error(`Timed out waiting for message after ${timeoutMs}ms. Received messages: ${JSON.stringify(messages)}`);
    }
    await new Promise((resolve) => setTimeout(resolve, 5));
  }
};

try {
  const ready = await waitFor((message) => message.type === 'ready');
  assert.ok(ready.compositions.includes('three_lifecycle_preview'));
  assert.ok(ready.compositions.includes('three_audio_reactive_preview'));
  const parityRequest = { type: 'render', composition: 'three_lifecycle_preview', frame: 1, fps: 30, width: 640, height: 360, props: { color: '#ff8844' } };
  child.stdin.write(`${JSON.stringify({ ...parityRequest, transport: 'rgba' })}\n`);
  const rgbaParity = await waitFor((message) => message.type === 'frame' && message.frame === 1);
  const rgbaBytes = Buffer.from(rgbaParity.video_frame.rgba_base64, 'base64');
  assert.equal(rgbaBytes.length, 640 * 360 * 4);
  child.stdin.write(`${JSON.stringify({ ...parityRequest, transport: 'rgba_file' })}\n`);
  const fileParity = await waitFor((message) => message.type === 'frame' && message.frame === 1);
  const fileBytes = readFileSync(fileParity.video_frame.file_path);
  assert.deepEqual(fileBytes, rgbaBytes);
  unlinkSync(fileParity.video_frame.file_path);
  for (const frame of [0, 15, 30]) {
    child.stdin.write(`${JSON.stringify({ type: 'render', composition: 'three_lifecycle_preview', frame, fps: 30, width: 640, height: 360, props: { color: '#ff8844' }, ...(frame === 0 ? { transport: 'rgba' } : frame === 15 ? { transport: 'rgba_file' } : {}) })}\n`);
    const response = await waitFor((message) => message.type === 'frame' && message.frame === frame);
    assert.equal(response.width, 640);
    assert.equal(response.height, 360);
    if (frame === 0) {
      assert.equal(response.video_frame.width, 640);
      assert.equal(response.video_frame.height, 360);
      assert.equal(Buffer.from(response.video_frame.rgba_base64, 'base64').length, 640 * 360 * 4);
    }
    if (frame === 15) {
      assert.equal(response.video_frame.width, 640);
      assert.equal(response.video_frame.height, 360);
      assert.match(response.video_frame.file_path, /dioxuscut-rgba-/);
      assert.equal(readFileSync(response.video_frame.file_path).length, 640 * 360 * 4);
      unlinkSync(response.video_frame.file_path);
    }
  }
  for (const frame of [0, 15, 30]) {
    child.stdin.write(`${JSON.stringify({ type: 'render', composition: 'three_audio_reactive_preview', frame, fps: 30, width: 640, height: 360 })}\n`);
    const response = await waitFor((message) => message.type === 'frame' && message.frame === frame);
    assert.equal(response.width, 640);
    assert.equal(response.height, 360);
  }
  console.log(JSON.stringify({ compositions: 2, frames: 8, raw_rgba_parity: true, status: 'ok' }));
} finally {
  child.stdin.write('{"type":"shutdown"}\n');
  await new Promise((resolve) => child.once('exit', resolve));
}
