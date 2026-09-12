//! Measure the persistent Chromium/Three.js worker without charging startup.
import {spawn} from 'node:child_process';
import {performance} from 'node:perf_hooks';
import {createInterface} from 'node:readline';

const root = new URL('../../', import.meta.url);
const worker = new URL('../../apps/studio-tauri/scripts/three-render-worker.mjs', import.meta.url);
const url = process.env.DIOXUSCUT_BROWSER_URL ?? 'http://127.0.0.1:1421';
const frames = Number(process.env.FRAMES ?? 180);
const repeats = Number(process.env.REPEATS ?? 3);
const node = process.env.DIOXUSCUT_BROWSER_NODE ?? process.execPath;
const browser = process.env.CHROME_PATH;

function run() {
  return new Promise((resolve, reject) => {
    const args = [worker.pathname, `--url=${url}`];
    if (browser) args.push(`--browser=${browser}`);
    const child = spawn(node, args, {stdio: ['pipe', 'pipe', 'inherit']});
    const lines = createInterface({input: child.stdout});
    const pending = new Map();
    let ready = false;
    let failed;
    lines.on('line', (line) => {
      try {
        const message = JSON.parse(line);
        if (message.type === 'ready') ready = true;
        if (message.type === 'frame' || message.type === 'error') {
          const item = pending.get(message.frame);
          if (item) {
            pending.delete(message.frame);
            message.type === 'error' ? item.reject(new Error(message.message)) : item.resolve();
          }
        }
      } catch (error) { failed = error; }
    });
    child.once('error', reject);
    child.once('exit', (code) => { if (code !== 0 && !failed) failed = new Error(`worker exited with ${code}`); });
    const waitReady = () => ready ? Promise.resolve() : new Promise((r, j) => setTimeout(() => waitReady().then(r, j), 5));
    const request = (frame) => new Promise((resolveFrame, rejectFrame) => {
      pending.set(frame, {resolve: resolveFrame, reject: rejectFrame});
      child.stdin.write(JSON.stringify({type: 'render', composition: 'three_preview', frame, fps: 30, width: 1280, height: 720, props: {}}) + '\n');
    });
    (async () => {
      await waitReady();
      if (failed) throw failed;
      const measure = async () => {
        const start = performance.now();
        for (let frame = 0; frame < frames; frame++) await request(frame);
        return performance.now() - start;
      };
      await measure();
      const samples = [];
      for (let i = 0; i < repeats; i++) samples.push(await measure());
      child.stdin.end(JSON.stringify({type: 'shutdown'}) + '\n');
      resolve(samples);
    })().catch((error) => { child.kill(); reject(error); });
  });
}

const samples = await run();
const sorted = [...samples].sort((a, b) => a - b);
console.log(JSON.stringify({backend: 'chromium-three-worker', url, frames, width: 1280, height: 720, repeats, samples_ms: samples, median_ms: sorted[Math.floor(sorted.length / 2)], fps_equivalent: frames / (sorted[Math.floor(sorted.length / 2)] / 1000)}, null, 2));
