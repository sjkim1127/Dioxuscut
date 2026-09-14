// Run from the repository root.
// "The Gauntlet": 1080p 30fps 900-frame (30s) heavy stress benchmark.
// 2x Video Streams, 20 Image cards, 60 Particle nodes, Sigma 30 Blur, Multilingual text.
import {bundle} from '@remotion/bundler';
import {openBrowser, renderMedia, selectComposition} from '@remotion/renderer';
import {execFileSync, spawn} from 'node:child_process';
import {mkdirSync, writeFileSync} from 'node:fs';
import {existsSync, readFileSync} from 'node:fs';
import path from 'node:path';
import os from 'node:os';
import {performance} from 'node:perf_hooks';

const root = process.cwd();
const outputDir = path.join(root, 'target/remotion-benchmark-gauntlet');
mkdirSync(outputDir, {recursive: true});

const nativeBinary = path.join(root, 'target/release/examples/gauntlet_bench');
const browserExecutable = process.env.CHROME_PATH ?? '/Applications/Google Chrome.app/Contents/MacOS/Google Chrome';

function ensureGauntletAssets() {
  const assetsDir = path.join(root, 'target/assets');
  mkdirSync(assetsDir, {recursive: true});
  const video = path.join(assetsDir, 'test_video.mp4');
  if (!existsSync(video)) {
    execFileSync('ffmpeg', [
      '-hide_banner', '-loglevel', 'error', '-y',
      '-f', 'lavfi', '-i', 'testsrc2=size=640x360:rate=30',
      '-t', '2', '-c:v', 'libx264', '-pix_fmt', 'yuv420p',
      '-movflags', '+faststart', video,
    ], {stdio: 'inherit'});
  }
  for (let i = 0; i < 20; i++) {
    const image = path.join(assetsDir, `img_${i.toString().padStart(2, '0')}.png`);
    if (existsSync(image)) continue;
    const r = 30 + i * 11;
    const g = 40 + i * 7;
    const b = 180 - i * 5;
    const color = `0x${[r, g, b].map((v) => v.toString(16).padStart(2, '0')).join('')}`;
    execFileSync('ffmpeg', [
      '-hide_banner', '-loglevel', 'error', '-y',
      '-f', 'lavfi', '-i', `color=c=${color}:s=256x256`,
      '-frames:v', '1', image,
    ], {stdio: 'inherit'});
  }
}

ensureGauntletAssets();
console.log('[*] Bundling Remotion Gauntlet scene...');
const setupStart = performance.now();
const serveUrl = await bundle({
  entryPoint: path.join(root, 'benchmarks/remotion/gauntlet_scene.tsx'),
  outDir: path.join(outputDir, 'bundle'),
});
const bundleMs = performance.now() - setupStart;
console.log(`[+] Bundle created in ${(bundleMs / 1000).toFixed(2)}s`);

console.log('[*] Launching Chromium browser...');
const browserStart = performance.now();
const browser = await openBrowser('chrome', {browserExecutable, logLevel: 'error'});
const browserMs = performance.now() - browserStart;
console.log(`[+] Browser launched in ${(browserMs / 1000).toFixed(2)}s`);

const common = {serveUrl, puppeteerInstance: browser, logLevel: 'error'};

try {
  const composition = await selectComposition({...common, id: 'GauntletScene'});

  console.log('[*] [ROUND 1] Rendering Remotion Gauntlet scene (1920x1080, 900 frames, concurrency: 4)...');
  const remotionStart = performance.now();
  await renderMedia({
    ...common,
    composition,
    outputLocation: path.join(outputDir, 'remotion-gauntlet.mp4'),
    codec: 'h264',
    crf: 18,
    x264Preset: 'fast',
    pixelFormat: 'yuv420p',
    concurrency: 4,
    imageFormat: 'png',
    muted: true,
    onProgress: ({renderedFrames}) => {
      if (renderedFrames % 150 === 0 && renderedFrames > 0) {
        console.log(`[*] [Remotion Progress] ${renderedFrames} / 900 frames (${(renderedFrames / 9).toFixed(1)}%)...`);
      }
    },
  });
  const remotionMs = performance.now() - remotionStart;
  console.log(`[+] Remotion Gauntlet render completed in ${(remotionMs / 1000).toFixed(2)}s (${(900 / (remotionMs / 1000)).toFixed(2)} FPS)`);

  console.log('[*] [ROUND 2] Rendering Dioxuscut Gauntlet scene (1920x1080, 900 frames, concurrency: 4)...');
  const dioxuscutStart = performance.now();
  const rawOutput = execFileSync(
    nativeBinary,
    [path.join(outputDir, 'dioxuscut-gauntlet.mp4'), '4', '900'],
    {encoding: 'utf8'}
  );
  const dioxuscutProcessMs = performance.now() - dioxuscutStart;
  const nativeStats = JSON.parse(rawOutput.trim());
  console.log(`[+] Dioxuscut Gauntlet render completed in ${(dioxuscutProcessMs / 1000).toFixed(2)}s (internal render: ${(nativeStats.render_ms / 1000).toFixed(2)}s, ${(900 / (dioxuscutProcessMs / 1000)).toFixed(2)} FPS)`);

  const ssimStatsPath = path.join(outputDir, 'gauntlet-ssim.log');
  execFileSync('ffmpeg', [
    '-hide_banner', '-loglevel', 'error',
    '-i', path.join(outputDir, 'remotion-gauntlet.mp4'),
    '-i', path.join(outputDir, 'dioxuscut-gauntlet.mp4'),
    '-lavfi', `ssim=stats_file=${ssimStatsPath}`,
    '-f', 'null', '-',
  ], {stdio: 'inherit'});
  const ssimValues = readFileSync(ssimStatsPath, 'utf8')
    .split('\n')
    .filter(Boolean)
    .map((line) => Number(/All:([0-9.]+)/.exec(line)?.[1]))
    .filter(Number.isFinite);
  const ssim = {
    frames: ssimValues.length,
    min: Math.min(...ssimValues),
    mean: ssimValues.reduce((sum, value) => sum + value, 0) / ssimValues.length,
  };
  console.log(`[+] Frame SSIM: min ${ssim.min.toFixed(6)}, mean ${ssim.mean.toFixed(6)} (${ssim.frames} frames)`);

  const speedup = remotionMs / dioxuscutProcessMs;
  console.log(`\n======================================================`);
  console.log(`🔥 THE GAUNTLET RESULT: Dioxuscut is ${speedup.toFixed(2)}x faster!`);
  console.log(`======================================================\n`);

  const report = {
    workload: 'The Gauntlet: 1080p 30fps 900-frame (30s) Heavy Stress Scene',
    features: [
      '2x Simultaneous Decoded MP4 Video Streams (PiP)',
      '20 High-Res PNG Image Cards with Sinusoidal Floating',
      '60 Floating Light Particle Orbs',
      'Heavy Glassmorphism Layer with Blur Sigma 30.0',
      'Multilingual Text (Korean, English, Symbols, Emojis)',
      '900 Total Frames (5x standard duration)',
    ],
    concurrency: 4,
    platform: `${os.platform()} (${os.arch()})`,
    cpu: os.cpus()[0].model,
    bundle_ms: bundleMs,
    browser_startup_ms: browserMs,
    remotion: {
      render_ms: remotionMs,
      fps: 900 / (remotionMs / 1000),
    },
    dioxuscut: {
      process_ms: dioxuscutProcessMs,
      render_ms: nativeStats.render_ms,
      fps: 900 / (dioxuscutProcessMs / 1000),
    },
    ssim,
    speedup,
  };

  const reportPath = path.join(root, 'benchmarks/gauntlet-bench-result.json');
  writeFileSync(reportPath, JSON.stringify(report, null, 2) + '\n');
  console.log(`[+] Full report saved to ${reportPath}`);
} finally {
  await browser.close({silent: true});
}
