import { createInterface } from 'node:readline';
import { Buffer } from 'node:buffer';
import { chromium } from 'playwright-core';

const args = new Map(process.argv.slice(2).flatMap((arg) => {
  const [key, value] = arg.split('=', 2);
  return key.startsWith('--') ? [[key.slice(2), value ?? '']] : [];
}));
const url = args.get('url') ?? 'http://localhost:1420';
const compositionModule = args.get('composition-module') ?? process.env.DIOXUSCUT_BROWSER_COMPOSITION_MODULE;
const executablePath = args.get('browser') ?? process.env.CHROME_PATH ??
  '/Applications/Google Chrome.app/Contents/MacOS/Google Chrome';
const configuredFrameTimeoutMs = Number(args.get('frame-timeout-ms') ?? process.env.DIOXUSCUT_BROWSER_FRAME_TIMEOUT_MS ?? 30000);
const frameTimeoutMs = Number.isFinite(configuredFrameTimeoutMs) && configuredFrameTimeoutMs > 0
  ? configuredFrameTimeoutMs
  : 30000;
const configuredFrameRetries = Number(args.get('frame-retries') ?? process.env.DIOXUSCUT_BROWSER_FRAME_RETRIES ?? 1);
const frameRetries = Number.isInteger(configuredFrameRetries) && configuredFrameRetries >= 0
  ? configuredFrameRetries
  : 1;
const write = (message) => process.stdout.write(`${JSON.stringify(message)}\n`);
const renderFrame = (request) => new Promise((resolve, reject) => {
  const timer = setTimeout(
    () => reject(new Error(`renderFrame timed out after ${frameTimeoutMs}ms`)),
    frameTimeoutMs,
  );
  page.evaluate((value) => window.dioxuscut.renderFrame(value), request)
    .then(
      (value) => {
        clearTimeout(timer);
        resolve(value);
      },
      (error) => {
        clearTimeout(timer);
        reject(error);
      },
    );
});

const browser = await chromium.launch({ executablePath, headless: true });
const page = await browser.newPage({ viewport: { width: 1280, height: 720 }, deviceScaleFactor: 1 });
await page.addInitScript(() => { window.__DIOXUSCUT_HEADLESS_RENDER__ = true; });
// Dev servers keep HMR/websocket connections open, so networkidle can never
// settle. The explicit renderFrame readiness check below is the real gate.
await page.goto(url, { waitUntil: 'domcontentloaded', timeout: frameTimeoutMs });
if (compositionModule) {
  await page.evaluate(async (moduleUrl) => {
    await import(moduleUrl);
  }, compositionModule);
}
await page.waitForFunction(() => typeof window.dioxuscut?.renderFrame === 'function', {
  timeout: frameTimeoutMs,
});
const compositions = await page.evaluate(() => window.dioxuscut.listCompositions?.() ?? []);
write({ type: 'ready', protocol: 1, compositions });

const rl = createInterface({ input: process.stdin, terminal: false });
let queue = Promise.resolve();
rl.on('line', (line) => { queue = queue.then(async () => {
  let message;
  try { message = JSON.parse(line); } catch { return write({ type: 'error', frame: null, message: 'invalid JSON' }); }
  if (message.type === 'shutdown') { await browser.close(); process.exit(0); }
  if (message.type !== 'render') return;
  try {
    const request = message;
    let pageError;
    const onPageError = (error) => { pageError = error; };
    page.on('pageerror', onPageError);
    let lastError;
    try {
      for (let attempt = 0; attempt <= frameRetries; attempt += 1) {
        try {
          await page.setViewportSize({ width: request.width, height: request.height });
          await renderFrame(request);
          if (pageError) throw pageError;
          lastError = undefined;
          break;
        } catch (error) {
          lastError = error;
        }
      }
    } finally {
      page.off('pageerror', onPageError);
    }
    if (lastError) throw lastError;
    const imageType = request.image_format === 'jpeg' ? 'jpeg' : 'png';
    const screenshot = await page.screenshot({ type: imageType,
      quality: imageType === 'jpeg' ? (request.jpeg_quality ?? 90) : undefined });
    write({ type: 'frame', frame: request.frame, width: request.width, height: request.height,
      ...(imageType === 'png'
        ? { png_base64: Buffer.from(screenshot).toString('base64') }
        : { jpeg_base64: Buffer.from(screenshot).toString('base64') }) });
  } catch (error) {
    write({ type: 'error', frame: message.frame ?? null, message: String(error) });
  }
}); });

process.once('SIGTERM', async () => { await browser.close(); process.exit(0); });
