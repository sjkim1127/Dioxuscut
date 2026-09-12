import { createRequire } from 'node:module';

const require = createRequire(new URL('../../apps/studio-tauri/package.json', import.meta.url));
const { chromium } = require('playwright-core');

const url = process.env.DIOXUSCUT_BROWSER_URL ?? 'http://127.0.0.1:1420';
const executablePath = process.env.CHROME_PATH ??
  '/Applications/Google Chrome.app/Contents/MacOS/Google Chrome';
const jsonAsset = new URL('/package.json', url).toString();
const canvasAsset = 'data:image/png;base64,iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAQAAAC1HAwCAAAAC0lEQVR42mNk+A8AAQUBAScY42YAAAAASUVORK5CYII=';
const retryAssetPath = '/canvas-retry.png';

const browser = await chromium.launch({ executablePath, headless: true });
try {
  const page = await browser.newPage({ viewport: { width: 320, height: 180 } });
  let retryRequests = 0;
  await page.route(`**${retryAssetPath}`, async (route) => {
    retryRequests += 1;
    if (retryRequests === 1) {
      await route.abort('failed');
      return;
    }
    await route.fulfill({
      status: 200,
      contentType: 'image/png',
      body: Buffer.from(canvasAsset.split(',')[1], 'base64'),
    });
  });
  await page.addInitScript(() => { window.__DIOXUSCUT_HEADLESS_RENDER__ = true; });
  await page.goto(url, { waitUntil: 'domcontentloaded' });
  await page.waitForFunction(() => typeof window.dioxuscut?.renderFrame === 'function');

  const result = await page.evaluate(async ({ jsonAsset, canvasAsset, retryAssetPath }) => {
    window.dioxuscut.registerComposition('media_sync_smoke', async ({ frame }) => {
      let media = document.querySelector('video[data-dioxuscut-smoke]');
      if (!media) {
        media = document.createElement('video');
        media.dataset.dioxuscutSmoke = '1';
        document.body.append(media);
      }

      let retryCanvas = document.querySelector('[data-dioxuscut-canvas-image-retry-smoke]');
      if (!retryCanvas) {
        retryCanvas = document.createElement('canvas');
        retryCanvas.dataset.dioxuscutCanvasImageRetrySmoke = '1';
        retryCanvas.dataset.dioxuscutCanvasImage = 'true';
        retryCanvas.dataset.src = new URL(retryAssetPath, location.href).toString();
        retryCanvas.dataset.fit = 'fill';
        retryCanvas.dataset.maxRetries = '1';
        retryCanvas.dataset.pauseWhenLoading = 'true';
        retryCanvas.setAttribute('width', '1');
        retryCanvas.setAttribute('height', '1');
        document.body.append(retryCanvas);
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

      let canvasImage = document.querySelector('[data-dioxuscut-canvas-image-smoke]');
      if (!canvasImage) {
        canvasImage = document.createElement('canvas');
        canvasImage.dataset.dioxuscutCanvasImageSmoke = '1';
        canvasImage.dataset.dioxuscutCanvasImage = 'true';
        canvasImage.dataset.src = canvasAsset;
        canvasImage.dataset.fit = 'contain';
        canvasImage.dataset.maxRetries = '2';
        canvasImage.dataset.pauseWhenLoading = 'true';
        canvasImage.setAttribute('width', '1');
        canvasImage.setAttribute('height', '1');
        document.body.append(canvasImage);
      }
    });

    await window.dioxuscut.renderFrame({
      composition: 'media_sync_smoke', frame: 15, fps: 10,
      props: {}, assets: [jsonAsset], timeline: [],
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
      props: {}, assets: [jsonAsset], timeline: [],
    });
    return {
      active,
      inactiveVisibility: media.style.visibility,
      lottieSvg: Boolean(lottie.querySelector('svg')),
      lottieFrame: lottie.dataset.frame,
      canvasPixel: [...document.querySelector('[data-dioxuscut-canvas-image-smoke]').getContext('2d').getImageData(0, 0, 1, 1).data],
      retryCanvasPixel: [...document.querySelector('[data-dioxuscut-canvas-image-retry-smoke]').getContext('2d').getImageData(0, 0, 1, 1).data],
      canvasRetries: document.querySelector('[data-dioxuscut-canvas-image-smoke]').dataset.maxRetries,
      canvasPause: document.querySelector('[data-dioxuscut-canvas-image-smoke]').dataset.pauseWhenLoading,
    };
  }, { jsonAsset, canvasAsset, retryAssetPath });

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
  assert(result.canvasPixel[3] > 0, `CanvasImage was not rasterized: ${result.canvasPixel}`);
  assert(result.canvasRetries === '2', `unexpected CanvasImage retries: ${result.canvasRetries}`);
  assert(result.canvasPause === 'true', 'CanvasImage loading policy was not preserved');
  assert(retryRequests === 2, `CanvasImage did not retry after failure: ${retryRequests}`);
  assert(result.retryCanvasPixel[3] > 0, `CanvasImage retry did not rasterize: ${result.retryCanvasPixel}`);
  console.log('browser media sync smoke: passed');
} finally {
  await browser.close();
}
