import assert from 'node:assert/strict';
import { createRequire } from 'node:module';
import { readFile, rm } from 'node:fs/promises';
import { execFile } from 'node:child_process';
import { promisify } from 'node:util';

const require = createRequire(new URL('../../apps/studio-tauri/package.json', import.meta.url));
const { chromium } = require('playwright-core');
const url = process.env.DIOXUSCUT_BROWSER_URL ?? 'http://127.0.0.1:1421';
const executablePath = process.env.CHROME_PATH ?? '/Applications/Google Chrome.app/Contents/MacOS/Google Chrome';
const run = promisify(execFile);
const audioFixturePath = `/tmp/dioxuscut-webcodecs-audio-${process.pid}.m4a`;
await run(process.env.FFMPEG_PATH ?? 'ffmpeg', [
  '-hide_banner', '-loglevel', 'error', '-f', 'lavfi', '-i', 'sine=frequency=440:duration=1',
  '-c:a', 'aac', '-b:a', '96k', '-ar', '48000', '-ac', '1', '-f', 'ipod', '-y', audioFixturePath,
]);
const browser = await chromium.launch({ executablePath, headless: true });
try {
  const page = await browser.newPage();
  const fixture = await readFile(new URL('../../assets/showcase.mp4', import.meta.url));
  const audioFixture = await readFile(audioFixturePath);
  const rangeFixture = Buffer.from([0, 1, 2, 3, 4, 5, 6, 7]);
  await page.route('**/range.bin', (route) => {
    const range = /^bytes=(\d+)-(\d+)$/.exec(route.request().headers().range ?? '');
    if (!range) return route.fulfill({ status: 416 });
    const start = Number(range[1]); const end = Number(range[2]);
    return route.fulfill({
      status: 206, contentType: 'application/octet-stream',
      headers: { 'Content-Range': `bytes ${start}-${end}/${rangeFixture.length}` },
      body: rangeFixture.subarray(start, end + 1),
    });
  });
  await page.route('**/assets/showcase.mp4', (route) => route.fulfill({
    status: 200, contentType: 'video/mp4', body: fixture,
  }));
  await page.route('**/assets/audio-fixture.m4a', (route) => route.fulfill({
    status: 200, contentType: 'audio/mp4', body: audioFixture,
  }));
  await page.addInitScript(() => { window.__DIOXUSCUT_HEADLESS_RENDER__ = true; });
  await page.goto(url, { waitUntil: 'domcontentloaded' });
  await page.waitForFunction(() => typeof window.dioxuscut?.createIsoBmffEncodedChunk === 'function');
  const result = await page.evaluate(async () => {
    const sample = {
      data: new Uint8Array([0, 0, 0, 1]),
      timestamp: 1.25,
      duration: 1 / 30,
      keyframe: true,
    };
    const chunk = window.dioxuscut.createIsoBmffEncodedChunk(sample);
    const config = window.dioxuscut.makeIsoBmffWebCodecsConfig({
      codecConfig: { type: 'avcC', data: new Uint8Array([1, 0x64, 0x00, 0x1f]) },
    });
    const annexB = window.dioxuscut.avccToAnnexB(new Uint8Array([0, 0, 0, 2, 0x65, 0x88]));
    const support = await VideoDecoder.isConfigSupported({ codec: 'avc1.42E01E' });
    const audio = window.dioxuscut.createIsoBmffEncodedChunk({
      data: new Uint8Array([0xff, 0xf1, 0x50, 0x80]), timestamp: 2, duration: 0.02,
    }, 'audio');
    const audioSupport = await AudioDecoder.isConfigSupported({ codec: 'mp4a.40.2', numberOfChannels: 2, sampleRate: 48000 });
    const parsed = await window.dioxuscut.parseIsoBmffMovieHeader('/assets/showcase.mp4');
    const parseEvents = [];
    const partialMetadata = await window.dioxuscut.parseMedia({
      src: '/assets/showcase.mp4',
      fields: { dimensions: true, durationInSeconds: true },
      onDimensions: (value) => parseEvents.push(['dimensions', value?.width ?? null]),
      onDurationInSeconds: (value) => parseEvents.push(['duration', value]),
      onContainer: (value) => parseEvents.push(['container', value]),
      onTracks: (value) => parseEvents.push(['tracks', value.length]),
    });
    if (!partialMetadata.dimensions || partialMetadata.durationInSeconds !== 3 || 'container' in partialMetadata) {
      throw new Error('parseMedia object fields selection failed');
    }
    const ranged = await window.dioxuscut.readMediaRange('/range.bin', 2, 6);
    const samples = await window.dioxuscut.readIsoBmffSamples('/assets/showcase.mp4', 0, [0, 1, 2], {
      parsed, concurrency: 2,
    });
    const codecConfig = window.dioxuscut.makeIsoBmffWebCodecsConfig(parsed.tracks[0]);
    const decodeStarted = performance.now();
    const frames = await window.dioxuscut.decodeIsoBmffVideo('/assets/showcase.mp4', {
      trackIndex: 0, startSample: 0, endSample: 3, maxSamples: 3,
      codec: codecConfig.codec, description: codecConfig.description, options: { parsed },
    });
    const videoDecodeMs = performance.now() - decodeStarted;
    const decoded = frames.map((frame) => ({ width: frame.displayWidth, height: frame.displayHeight, timestamp: frame.timestamp }));
    for (const frame of frames) frame.close();
    const streamedVideo = [];
    const streamedVideoResult = await window.dioxuscut.decodeIsoBmffVideo('/assets/showcase.mp4', {
      trackIndex: 0, startSample: 0, endSample: 3, maxSamples: 3,
      codec: codecConfig.codec, description: codecConfig.description, options: { parsed },
      onFrame: (frame) => { streamedVideo.push(frame.timestamp); frame.close(); },
    });
    const audioParsed = await window.dioxuscut.parseIsoBmffMovieHeader('/assets/audio-fixture.m4a');
    const audioConfig = window.dioxuscut.makeIsoBmffWebCodecsConfig(audioParsed.tracks[0]);
    const audioDecodeStarted = performance.now();
    const audioData = await window.dioxuscut.decodeIsoBmffAudio('/assets/audio-fixture.m4a', {
      trackIndex: 0, startSample: 0, endSample: 3, maxSamples: 3,
      codec: audioConfig.codec, description: audioConfig.description,
      numberOfChannels: 1, sampleRate: 48000, options: { parsed: audioParsed },
    });
    const audioDecodeMs = performance.now() - audioDecodeStarted;
    const decodedAudio = audioData.map((audio) => ({ frames: audio.numberOfFrames, timestamp: audio.timestamp }));
    for (const audio of audioData) audio.close();
    const streamedAudio = [];
    const streamedAudioResult = await window.dioxuscut.decodeIsoBmffAudio('/assets/audio-fixture.m4a', {
      trackIndex: 0, startSample: 0, endSample: 3, maxSamples: 3,
      codec: audioConfig.codec, description: audioConfig.description,
      numberOfChannels: 1, sampleRate: 48000, options: { parsed: audioParsed },
      onAudioData: (audio) => { streamedAudio.push(audio.timestamp); audio.close(); },
    });
    return {
      type: chunk.type, timestamp: chunk.timestamp, duration: chunk.duration, supported: support.supported,
      annexB: [...annexB],
      codec: config.codec,
      codecFormat: config.format,
      descriptionLength: config.description.byteLength,
      audioTimestamp: audio.timestamp, audioDuration: audio.duration, audioSupported: audioSupport.supported,
      container: parsed.container,
      boxes: parsed.boxes.map(({ type }) => type),
      mediaDuration: parsed.durationInSeconds,
      sampleCount: parsed.tracks[0].sampleTables.sampleRanges.length,
      codecConfig: parsed.tracks[0].codecConfig.type,
      batchSamples: samples.map(({ sampleIndex, offset, size, timestamp, keyframe, data }) => ({
        sampleIndex, offset, size, timestamp, keyframe, bytes: data.byteLength,
        sampleDescriptionIndex: parsed.tracks[0].sampleTables.sampleRanges[sampleIndex].sampleDescriptionIndex,
      })),
      ranged: [...ranged],
      parseEvents,
      decoded,
      videoDecodeMs,
      decodedAudio,
      audioDecodeMs,
      streamedVideo: { count: streamedVideo.length, returnValue: streamedVideoResult },
      streamedAudio: { count: streamedAudio.length, returnValue: streamedAudioResult },
    };
  });
  assert.equal(result.type, 'key');
  assert.equal(result.timestamp, 1_250_000);
  assert.equal(result.duration, Math.round(1_000_000 / 30));
  assert.equal(result.supported, true);
  assert.deepEqual(result.annexB, [0, 0, 0, 1, 0x65, 0x88]);
  assert.equal(result.codec, 'avc1.64001f');
  assert.equal(result.codecFormat, 'avc');
  assert.equal(result.descriptionLength, 4);
  assert.equal(result.audioTimestamp, 2_000_000);
  assert.equal(result.audioDuration, 20_000);
  assert.equal(result.audioSupported, true);
  assert.equal(result.container, 'iso-base-media');
  assert.deepEqual(result.boxes, ['ftyp', 'moov', 'free', 'mdat']);
  assert.equal(result.mediaDuration, 3);
  assert.equal(result.sampleCount, 180);
  assert.equal(result.codecConfig, 'avcC');
  assert.equal(result.batchSamples.length, 3);
  assert.deepEqual(result.batchSamples.map(({ sampleIndex }) => sampleIndex), [0, 1, 2]);
  assert.equal(result.batchSamples[0].timestamp, 0);
  assert.equal(result.batchSamples[0].keyframe, true);
  assert.equal(result.batchSamples[0].sampleDescriptionIndex, 1);
  assert.ok(result.batchSamples.every(({ size, bytes }) => size === bytes && size > 0));
  assert.deepEqual(result.ranged, [2, 3, 4, 5]);
  assert.equal(result.decoded.length, 3);
  assert.ok(result.decoded.every(({ width, height }) => width > 0 && height > 0));
  assert.ok(result.decoded.every(({ timestamp }, index, frames) => index === 0 || timestamp > frames[index - 1].timestamp));
  assert.ok(result.decodedAudio.length > 0);
  assert.ok(result.decodedAudio.every(({ frames }) => frames > 0));
  assert.equal(result.streamedVideo.count, 3);
  assert.equal(result.streamedVideo.returnValue, null);
  assert.ok(result.streamedAudio.count > 0);
  assert.equal(result.streamedAudio.returnValue, null);
  assert.ok(result.videoDecodeMs >= 0 && result.audioDecodeMs >= 0);
  console.log(JSON.stringify({ status: 'ok', ...result }));
} finally {
  await browser.close();
  await rm(audioFixturePath, { force: true });
}
