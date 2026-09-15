import { spawn } from 'node:child_process';
import { createInterface } from 'node:readline';
import { fileURLToPath, pathToFileURL } from 'node:url';
import { dirname, resolve } from 'node:path';

const here = dirname(fileURLToPath(import.meta.url));
const worker = resolve(here, 'three-render-worker.mjs');
const fixture = pathToFileURL(resolve(here, 'webcodecs-roundtrip-fixture.html')).href;
const child = spawn(process.execPath, [worker, `--url=${fixture}`], {
  stdio: ['pipe', 'pipe', 'inherit'],
});
const lines = createInterface({ input: child.stdout });
const nextMessage = () => new Promise((resolveMessage, reject) => {
  const onLine = (line) => {
    try {
      resolveMessage(JSON.parse(line));
    } catch (error) {
      reject(new Error(`worker emitted invalid JSON: ${error.message}`));
    }
    lines.off('line', onLine);
  };
  lines.on('line', onLine);
  child.once('error', reject);
});

try {
  const ready = await nextMessage();
  if (ready.type !== 'ready' || !ready.compositions.includes('roundtrip')) {
    throw new Error(`unexpected worker handshake: ${JSON.stringify(ready)}`);
  }
  child.stdin.write(JSON.stringify({
    type: 'render', frame: 1, fps: 30, width: 4, height: 2, transport: 'rgba',
  }) + '\n');
  const response = await nextMessage();
  if (response.type !== 'frame' || response.transport !== 'webcodecs') {
    throw new Error(`WebCodecs transport was not used: ${JSON.stringify(response)}`);
  }
  const videoFrame = response.video_frame;
  const rgba = Buffer.from(videoFrame.rgba_base64, 'base64');
  if (videoFrame.width !== 4 || videoFrame.height !== 2 || rgba.length !== 32) {
    throw new Error(`invalid RGBA frame payload: ${JSON.stringify(response)}`);
  }
  if (videoFrame.timestamp_us !== 33333) {
    throw new Error(`invalid timestamp: ${videoFrame.timestamp_us}`);
  }
  console.log('WebCodecs worker round-trip: ok');
} finally {
  child.stdin.write('{"type":"shutdown"}\n');
  await new Promise((resolveExit) => child.once('exit', resolveExit));
}
