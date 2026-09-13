import { createInterface } from 'node:readline';

process.stdout.write('{"type":"ready","protocol":1}\n');
const input = createInterface({ input: process.stdin, terminal: false });
input.on('line', (line) => {
  const message = JSON.parse(line);
  if (message.type === 'shutdown') process.exit(0);
  if (message.type !== 'render') return;
  const timestampUs = Math.round((message.frame / message.fps) * 1_000_000);
  process.stdout.write(JSON.stringify({
    type: 'frame',
    frame: message.frame,
    width: message.width,
    height: message.height,
    video_frame: {
      width: message.width,
      height: message.height,
      timestamp_us: timestampUs,
      rgba_base64: 'AQIDBA==',
    },
  }) + '\n');
});
