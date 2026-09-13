import { createInterface } from 'node:readline';
import { Buffer } from 'node:buffer';
import { existsSync } from 'node:fs';
import { writeFileSync, unlinkSync } from 'node:fs';
import { tmpdir } from 'node:os';
import { chromium } from 'playwright-core';

const args = new Map(process.argv.slice(2).flatMap((arg) => {
  const [key, value] = arg.split('=', 2);
  return key.startsWith('--') ? [[key.slice(2), value ?? '']] : [];
}));
const url = args.get('url') ?? 'http://localhost:1420';
const compositionModule = args.get('composition-module') ?? process.env.DIOXUSCUT_BROWSER_COMPOSITION_MODULE;
const executablePath = args.get('browser') ?? process.env.CHROME_PATH ?? findBrowserExecutable();
const configuredFrameTimeoutMs = Number(args.get('frame-timeout-ms') ?? process.env.DIOXUSCUT_BROWSER_FRAME_TIMEOUT_MS ?? 30000);
const frameTimeoutMs = Number.isFinite(configuredFrameTimeoutMs) && configuredFrameTimeoutMs > 0
  ? configuredFrameTimeoutMs
  : 30000;
const configuredFrameRetries = Number(
  args.get('frame-retries')
    ?? process.env.DIOXUSCUT_BROWSER_TRANSPORT_RETRIES
    ?? process.env.DIOXUSCUT_BROWSER_FRAME_RETRIES
    ?? 1,
);
const frameRetries = Number.isInteger(configuredFrameRetries) && configuredFrameRetries >= 0
  ? configuredFrameRetries
  : 1;
const write = (message) => process.stdout.write(`${JSON.stringify(message)}\n`);
const pendingFrameFiles = new Set();
const cleanupFrameFiles = () => {
  for (const path of pendingFrameFiles) {
    try { unlinkSync(path); } catch {}
  }
  pendingFrameFiles.clear();
};
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

const browser = await chromium.launch({ ...(executablePath ? { executablePath } : {}), headless: true });
const page = await browser.newPage({ viewport: { width: 1280, height: 720 }, deviceScaleFactor: 1 });
let viewportWidth = 1280;
let viewportHeight = 720;
await page.addInitScript(() => { window.__DIOXUSCUT_HEADLESS_RENDER__ = true; });
// Dev servers keep HMR/websocket connections open, so networkidle can never
// settle. The explicit renderFrame readiness check below is the real gate.
await page.goto(url, { waitUntil: 'domcontentloaded', timeout: frameTimeoutMs });
if (compositionModule) {
  await page.evaluate(async (moduleUrl) => {
    const loadedModule = await eval('import(moduleUrl)');
    const api = window.dioxuscut;
    if (typeof loadedModule.register === 'function') await loadedModule.register(api);
    const entries = loadedModule.compositions ?? loadedModule.default;
    if (entries && typeof entries === 'object' && !Array.isArray(entries)) {
      for (const [id, render] of Object.entries(entries)) {
        if (typeof render === 'function') api.registerComposition(id, render);
      }
    }
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
  if (message.type === 'shutdown') { cleanupFrameFiles(); await browser.close(); process.exit(0); }
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
          if (request.width !== viewportWidth || request.height !== viewportHeight) {
            await page.setViewportSize({ width: request.width, height: request.height });
            viewportWidth = request.width;
            viewportHeight = request.height;
          }
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
    const fileTransport = request.transport === 'file';
    const screenshot = await page.screenshot({ type: imageType, ...(fileTransport ? {} : { encoding: 'base64' }),
      omitBackground: imageType === 'png' && request.transparent === true,
      quality: imageType === 'jpeg' ? (request.jpeg_quality ?? 90) : undefined });
    if (fileTransport) {
      const path = `${tmpdir()}/dioxuscut-frame-${process.pid}-${request.frame}-${Date.now()}.${imageType}`;
      writeFileSync(path, screenshot);
      pendingFrameFiles.add(path);
      write({ type: 'frame', frame: request.frame, width: request.width, height: request.height, file_path: path });
    } else {
    write({ type: 'frame', frame: request.frame, width: request.width, height: request.height,
      ...(imageType === 'png'
        ? { png_base64: screenshot }
        : { jpeg_base64: screenshot }) });
    }
  } catch (error) {
    write({ type: 'error', frame: message.frame ?? null, message: String(error) });
  }
}); });

process.once('SIGTERM', async () => { cleanupFrameFiles(); await browser.close(); process.exit(0); });

function findBrowserExecutable() {
  const candidates = process.platform === 'darwin'
    ? ['/Applications/Google Chrome.app/Contents/MacOS/Google Chrome',
      '/Applications/Chromium.app/Contents/MacOS/Chromium', 'google-chrome', 'chromium']
    : process.platform === 'win32'
      ? [
        `${process.env.PROGRAMFILES ?? 'C:\\Program Files'}\\Google\\Chrome\\Application\\chrome.exe`,
        `${process.env['PROGRAMFILES(X86)'] ?? 'C:\\Program Files (x86)'}\\Google\\Chrome\\Application\\chrome.exe`,
        `${process.env.LOCALAPPDATA ?? ''}\\Google\\Chrome\\Application\\chrome.exe`,
        'chrome.exe',
      ]
      : ['google-chrome', 'google-chrome-stable', 'chromium', 'chromium-browser'];
  const pathEntries = (process.env.PATH ?? '').split(process.platform === 'win32' ? ';' : ':');
  for (const candidate of candidates) {
    if (existsSync(candidate)) return candidate;
    if (!candidate.includes('/') && !candidate.includes('\\')) {
      const onPath = pathEntries.map((entry) => `${entry}/${candidate}`).find(existsSync);
      if (onPath) return onPath;
    }
  }
  return undefined;
}
