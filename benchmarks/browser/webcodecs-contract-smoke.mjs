import assert from 'node:assert/strict';
import { createRequire } from 'node:module';

const require = createRequire(new URL('../../apps/studio-tauri/package.json', import.meta.url));
const { chromium } = require('playwright-core');
const url = process.env.DIOXUSCUT_BROWSER_URL ?? 'http://127.0.0.1:1421';
const executablePath = process.env.CHROME_PATH ?? '/Applications/Google Chrome.app/Contents/MacOS/Google Chrome';
const browser = await chromium.launch({ executablePath, headless: true });
try {
  const page = await browser.newPage();
  await page.addInitScript(() => { window.__DIOXUSCUT_HEADLESS_RENDER__ = true; });
  await page.goto(url, { waitUntil: 'domcontentloaded' });
  await page.waitForFunction(() => typeof window.dioxuscut?.createIsoBmffEncodedChunk === 'function');
  const result = await page.evaluate(async () => {
    const sample = {
      data: new Uint8Array([0, 0, 0, 1]),
      timestamp: 1.25,
      duration: 1 / 30,
      keyframe: true,
    };
    const chunk = window.dioxuscut.createIsoBmffEncodedChunk(sample);
    const support = await VideoDecoder.isConfigSupported({ codec: 'avc1.42E01E' });
    const audio = window.dioxuscut.createIsoBmffEncodedChunk({
      data: new Uint8Array([0xff, 0xf1, 0x50, 0x80]), timestamp: 2, duration: 0.02,
    }, 'audio');
    const audioSupport = await AudioDecoder.isConfigSupported({ codec: 'mp4a.40.2', numberOfChannels: 2, sampleRate: 48000 });
    return {
      type: chunk.type, timestamp: chunk.timestamp, duration: chunk.duration, supported: support.supported,
      audioTimestamp: audio.timestamp, audioDuration: audio.duration, audioSupported: audioSupport.supported,
    };
  });
  assert.equal(result.type, 'key');
  assert.equal(result.timestamp, 1_250_000);
  assert.equal(result.duration, Math.round(1_000_000 / 30));
  assert.equal(result.supported, true);
  assert.equal(result.audioTimestamp, 2_000_000);
  assert.equal(result.audioDuration, 20_000);
  assert.equal(result.audioSupported, true);
  console.log(JSON.stringify({ status: 'ok', ...result }));
} finally {
  await browser.close();
}
