import { chromium } from 'playwright-core';

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
    });

    await window.dioxuscut.renderFrame({
      composition: 'media_sync_smoke', frame: 15, fps: 10,
      props: {}, assets: [], timeline: [],
    });
    const media = document.querySelector('video[data-dioxuscut-smoke]');
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
    return { active, inactiveVisibility: media.style.visibility };
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
  console.log('browser media sync smoke: passed');
} finally {
  await browser.close();
}
