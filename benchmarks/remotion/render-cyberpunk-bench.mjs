// Run from the repository root.
// Compares Remotion vs Dioxuscut on a realistic 1080p Cyberpunk motion graphics scene
// exercising text auto-fitting, drop-shadow glow, offscreen layers, chromatic aberration, and vignette.
import {bundle} from '@remotion/bundler';
import {openBrowser, renderMedia, selectComposition} from '@remotion/renderer';
import {execFileSync} from 'node:child_process';
import {mkdirSync, writeFileSync} from 'node:fs';
import path from 'node:path';
import os from 'node:os';
import {performance} from 'node:perf_hooks';

const root = process.cwd();
const outputDir = path.join(root, 'target/remotion-benchmark-cyberpunk');
mkdirSync(outputDir, {recursive: true});

const nativeBinary = path.join(root, 'target/release/examples/cyberpunk_bench');
const browserExecutable = process.env.CHROME_PATH ?? '/Applications/Google Chrome.app/Contents/MacOS/Google Chrome';

console.log('[*] Bundling Remotion Cyberpunk scene...');
const setupStart = performance.now();
const serveUrl = await bundle({
  entryPoint: path.join(root, 'benchmarks/remotion/cyberpunk_scene.tsx'),
  outDir: path.join(outputDir, 'bundle'),
});
const bundleMs = performance.now() - setupStart;

console.log('[*] Launching Chromium browser...');
const browserStart = performance.now();
const browser = await openBrowser('chrome', {browserExecutable, logLevel: 'error'});
const browserMs = performance.now() - browserStart;

const common = {serveUrl, puppeteerInstance: browser, logLevel: 'error'};

try {
  const composition = await selectComposition({...common, id: 'CyberpunkScene'});

  console.log('[*] Rendering Remotion Cyberpunk scene (1920x1080, 180 frames, concurrency: 4)...');
  const remotionStart = performance.now();
  await renderMedia({
    ...common,
    composition,
    outputLocation: path.join(outputDir, 'remotion-cyberpunk.mp4'),
    codec: 'h264',
    crf: 18,
    x264Preset: 'fast',
    pixelFormat: 'yuv420p',
    concurrency: 4,
    imageFormat: 'png',
    muted: true,
  });
  const remotionMs = performance.now() - remotionStart;
  console.log(`[+] Remotion render completed in ${(remotionMs / 1000).toFixed(2)}s`);

  console.log('[*] Rendering Dioxuscut Cyberpunk scene (1920x1080, 180 frames, concurrency: 4)...');
  const dioxuscutStart = performance.now();
  const rawOutput = execFileSync(nativeBinary, [path.join(outputDir, 'dioxuscut-cyberpunk.mp4'), '4'], {
    encoding: 'utf8',
  });
  const dioxuscutProcessMs = performance.now() - dioxuscutStart;
  const nativeStats = JSON.parse(rawOutput.trim());
  console.log(`[+] Dioxuscut render completed in ${(dioxuscutProcessMs / 1000).toFixed(2)}s (internal render: ${(nativeStats.render_ms / 1000).toFixed(2)}s)`);

  const speedup = remotionMs / dioxuscutProcessMs;
  console.log(`\n======================================================`);
  console.log(`🔥 1080p VFX Speedup: Dioxuscut is ${speedup.toFixed(2)}x faster!`);
  console.log(`======================================================\n`);

  const report = {
    workload: '1080p Cyberpunk Title Reel (1920x1080, 30fps, 180 frames)',
    features: ['Text Auto-Fitting', 'Drop-Shadow Glow', 'Chromatic Aberration', 'Vignette', 'Layer Composition'],
    concurrency: 4,
    platform: `${os.platform()} (${os.arch()})`,
    cpu: os.cpus()[0].model,
    bundle_ms: bundleMs,
    browser_startup_ms: browserMs,
    remotion: {
      render_ms: remotionMs,
      fps: 180 / (remotionMs / 1000),
    },
    dioxuscut: {
      process_ms: dioxuscutProcessMs,
      render_ms: nativeStats.render_ms,
      fps: 180 / (dioxuscutProcessMs / 1000),
    },
    speedup,
  };

  const reportPath = path.join(root, 'benchmarks/cyberpunk-bench-result.json');
  writeFileSync(reportPath, JSON.stringify(report, null, 2) + '\n');
  console.log(`[+] Report saved to ${reportPath}`);
} finally {
  await browser.close();
}
