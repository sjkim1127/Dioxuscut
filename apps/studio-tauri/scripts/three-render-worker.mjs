import { createInterface } from 'node:readline';
import { Buffer } from 'node:buffer';
import { PNG } from 'pngjs';
import { chromium } from 'playwright-core';

const args = new Map(process.argv.slice(2).flatMap((arg) => {
  const [key, value] = arg.split('=', 2);
  return key.startsWith('--') ? [[key.slice(2), value ?? '']] : [];
}));
const url = args.get('url') ?? 'http://localhost:1420';
const executablePath = args.get('browser') ?? process.env.CHROME_PATH ??
  '/Applications/Google Chrome.app/Contents/MacOS/Google Chrome';
const write = (message) => process.stdout.write(`${JSON.stringify(message)}\n`);

const browser = await chromium.launch({ executablePath, headless: true });
const page = await browser.newPage({ viewport: { width: 1280, height: 720 }, deviceScaleFactor: 1 });
await page.goto(url, { waitUntil: 'networkidle' });
write({ type: 'ready', protocol: 1 });

const rl = createInterface({ input: process.stdin, terminal: false });
let queue = Promise.resolve();
rl.on('line', (line) => { queue = queue.then(async () => {
  let message;
  try { message = JSON.parse(line); } catch { return write({ type: 'error', frame: null, message: 'invalid JSON' }); }
  if (message.type === 'shutdown') { await browser.close(); process.exit(0); }
  if (message.type !== 'render') return;
  try {
    const request = message;
    await page.evaluate((value) => window.dioxuscut?.renderFrame(value), request);
    const png = PNG.sync.read(await page.screenshot({ type: 'png' }));
    write({ type: 'frame', frame: request.frame, width: png.width, height: png.height,
      rgba_base64: Buffer.from(png.data).toString('base64') });
  } catch (error) {
    write({ type: 'error', frame: message.frame ?? null, message: String(error) });
  }
}); });

process.once('SIGTERM', async () => { await browser.close(); process.exit(0); });
