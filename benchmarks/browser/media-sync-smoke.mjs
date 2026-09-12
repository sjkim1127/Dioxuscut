import { createRequire } from 'node:module';

const require = createRequire(new URL('../../apps/studio-tauri/package.json', import.meta.url));
const { chromium } = require('playwright-core');

const url = process.env.DIOXUSCUT_BROWSER_URL ?? 'http://127.0.0.1:1420';
const executablePath = process.env.CHROME_PATH ??
  '/Applications/Google Chrome.app/Contents/MacOS/Google Chrome';

const browser = await chromium.launch({ executablePath, headless: true });
try {
  const page = await browser.newPage({ viewport: { width: 320, height: 180 } });
  await page.addInitScript(() => { window.__DIOXUSCUT_HEADLESS_RENDER__ = true; });
  await page.goto(url, { waitUntil: 'domcontentloaded' });
  await page.waitForFunction(() => typeof window.dioxuscut?.renderFrame === 'function');

  const result = await page.evaluate(async () => {
    window.dioxuscut.registerComposition('media_sync_smoke', async ({ frame }) => {
      let media = document.querySelector('video[data-dioxuscut-smoke]');
      if (!media) {
        media = document.createElement('video');
        media.dataset.dioxuscutSmoke = '1';
        document.body.append(media);
      }
      media.dataset.time = String(frame / 10);
      media.dataset.timelineStart = '1';
      media.dataset.duration = '2';
      media.dataset.volume = '0.35';
      media.dataset.playbackRate = '1.5';
      media.setAttribute('loop', '');

      let lottie = document.querySelector('[data-dioxuscut-lottie-smoke]');
      if (!lottie) {
        lottie = document.createElement('div');
        lottie.dataset.dioxuscutLottieSmoke = '1';
        lottie.dataset.dioxuscutLottie = `data:application/json,${encodeURIComponent(JSON.stringify({
          v: '5.5.7', fr: 30, ip: 0, op: 30, w: 16, h: 16, nm: 'smoke', ddd: 0,
          assets: [], layers: [],
        }))}`;
        lottie.dataset.playbackRate = '1';
        lottie.dataset.time = String(frame / 10);
        lottie.dataset.loop = 'Loop';
        document.body.append(lottie);
      }
      lottie.dataset.time = String(frame / 10);
    });

    await window.dioxuscut.renderFrame({
      composition: 'media_sync_smoke', frame: 15, fps: 10,
      props: {}, assets: [], timeline: [],
    });
    const media = document.querySelector('video[data-dioxuscut-smoke]');
    const lottie = document.querySelector('[data-dioxuscut-lottie-smoke]');
    const active = {
      time: media.currentTime,
      visibility: media.style.visibility,
      volume: media.volume,
      rate: media.playbackRate,
      loop: media.loop,
    };

    await window.dioxuscut.renderFrame({
      composition: 'media_sync_smoke', frame: 40, fps: 10,
      props: {}, assets: [], timeline: [],
    });
    return {
      active,
      inactiveVisibility: media.style.visibility,
      lottieSvg: Boolean(lottie.querySelector('svg')),
      lottieFrame: lottie.dataset.frame,
    };
  });

  const assert = (condition, message) => {
    if (!condition) throw new Error(message);
  };
  assert(Math.abs(result.active.time - 1.5) < 0.001, `unexpected currentTime: ${result.active.time}`);
  assert(result.active.visibility === '', `active media is hidden: ${result.active.visibility}`);
  assert(Math.abs(result.active.volume - 0.35) < 0.001, `unexpected volume: ${result.active.volume}`);
  assert(Math.abs(result.active.rate - 1.5) < 0.001, `unexpected playbackRate: ${result.active.rate}`);
  assert(result.active.loop, 'loop was not enabled');
  assert(result.inactiveVisibility === 'hidden', 'inactive media was not hidden');
  assert(result.lottieSvg, 'Lottie adapter did not create an SVG');
  assert(result.lottieFrame === '40', `unexpected Lottie frame: ${result.lottieFrame}`);
  console.log('browser media sync smoke: passed');
} finally {
  await browser.close();
}
