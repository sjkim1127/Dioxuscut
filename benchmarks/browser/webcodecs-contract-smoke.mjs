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
const webmFixturePath = `/tmp/dioxuscut-webcodecs-video-${process.pid}.webm`;
const webmAudioFixturePath = `/tmp/dioxuscut-webcodecs-audio-${process.pid}.webm`;
await run(process.env.FFMPEG_PATH ?? 'ffmpeg', [
  '-hide_banner', '-loglevel', 'error', '-f', 'lavfi', '-i', 'sine=frequency=440:duration=1',
  '-c:a', 'aac', '-b:a', '96k', '-ar', '48000', '-ac', '1', '-f', 'ipod', '-y', audioFixturePath,
]);
await run(process.env.FFMPEG_PATH ?? 'ffmpeg', [
  '-hide_banner', '-loglevel', 'error', '-f', 'lavfi', '-i', 'testsrc2=size=160x90:rate=24:duration=1',
  '-c:v', 'libvpx-vp9', '-crf', '35', '-b:v', '0', '-an', '-y', webmFixturePath,
]);
await run(process.env.FFMPEG_PATH ?? 'ffmpeg', [
  '-hide_banner', '-loglevel', 'error', '-f', 'lavfi', '-i', 'sine=frequency=440:duration=1',
  '-c:a', 'libopus', '-b:a', '64k', '-ar', '48000', '-ac', '1', '-vn', '-y', webmAudioFixturePath,
]);
const browser = await chromium.launch({ executablePath, headless: true });
try {
  const page = await browser.newPage();
  const fixture = await readFile(new URL('../../assets/showcase.mp4', import.meta.url));
  const audioFixture = await readFile(audioFixturePath);
  const webmFixture = await readFile(webmFixturePath);
  const webmAudioFixture = await readFile(webmAudioFixturePath);
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
  await page.route('**/assets/webm-fixture.webm', (route) => route.fulfill({
    status: 200, contentType: 'video/webm', body: webmFixture,
  }));
  await page.route('**/assets/webm-audio-fixture.webm', (route) => route.fulfill({
    status: 200, contentType: 'audio/webm', body: webmAudioFixture,
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
    const webmMetadata = await window.dioxuscut.parseMedia({ src: '/assets/webm-fixture.webm' });
    const webmSamples = await window.dioxuscut.readWebmSamples('/assets/webm-fixture.webm', 1);
    const webmChunks = webmSamples.slice(0, 2).map((sample) =>
      window.dioxuscut.createWebmEncodedVideoChunk(sample, 1 / 24));
    const webmFrames = await window.dioxuscut.decodeWebmVideo('/assets/webm-fixture.webm', {
      trackNumber: 1, maxSamples: 3, codec: 'vp09.00.10.08',
    });
    const webmDecodedDimensions = webmFrames.map(({ displayWidth, displayHeight }) => [displayWidth, displayHeight]);
    for (const frame of webmFrames) frame.close();
    const webmAudioSamples = await window.dioxuscut.readWebmSamples('/assets/webm-audio-fixture.webm', 1);
    const webmAudioMetadata = await window.dioxuscut.parseMedia({ src: '/assets/webm-audio-fixture.webm' });
    const webmAudioData = await window.dioxuscut.decodeWebmAudio('/assets/webm-audio-fixture.webm', {
      trackNumber: 1, maxSamples: 3,
    });
    const webmAudioFrames = webmAudioData.map(({ numberOfFrames }) => numberOfFrames);
    for (const audio of webmAudioData) audio.close();
    let streamedWebmFrames = 0;
    const streamedWebmResult = await window.dioxuscut.decodeWebmVideo('/assets/webm-fixture.webm', {
      trackNumber: 1, maxSamples: 3, codec: 'vp09.00.10.08',
      onFrame: (frame) => { streamedWebmFrames += 1; frame.close(); },
    });
    const parseEvents = [];
    const partialMetadata = await window.dioxuscut.parseMedia({
      src: '/assets/showcase.mp4',
      fields: { dimensions: true, durationInSeconds: true },
      onDimensions: (value) => parseEvents.push(['dimensions', value?.width ?? null]),
      onDurationInSeconds: (value) => parseEvents.push(['duration', value]),
      onFps: (value) => parseEvents.push(['fps', Math.round(value)]),
      onVideoTrack: (value) => parseEvents.push(['videoTrack', value.width]),
      onVideoCodec: (value) => parseEvents.push(['videoCodec', value]),
      onKeyframes: (value) => parseEvents.push(['keyframes', value[0]?.positionInBytes, value[0]?.trackId]),
      onContainer: (value) => parseEvents.push(['container', value]),
      onTracks: (value) => parseEvents.push(['tracks', value.length]),
    });
    if (!partialMetadata.dimensions || partialMetadata.durationInSeconds !== 3 || 'container' in partialMetadata
      || !parseEvents.some(([name, value]) => name === 'fps' && value === 60)
      || !parseEvents.some(([name, value]) => name === 'videoTrack' && value === 1920)
      || !parseEvents.some(([name, value]) => name === 'videoCodec' && value.startsWith('avc1.'))
      || !parseEvents.some(([name, value, trackId]) => name === 'keyframes' && value === 3012 && trackId === 1)) {
      throw new Error(`parseMedia object fields selection failed: ${JSON.stringify({ partialMetadata, parseEvents })}`);
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
    const audioMetadataEvents = [];
    await window.dioxuscut.parseMedia({
      src: '/assets/audio-fixture.m4a',
      onSampleRate: (value) => audioMetadataEvents.push(['rate', value]),
      onNumberOfAudioChannels: (value) => audioMetadataEvents.push(['channels', value]),
    });
    if (!audioMetadataEvents.some(([name, value]) => name === 'rate' && value === 48000)
      || !audioMetadataEvents.some(([name, value]) => name === 'channels' && value === 1)) {
      throw new Error(`AAC metadata callbacks failed: ${JSON.stringify(audioParsed.tracks?.[0]?.codecConfig)}`);
    }
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
      webm: {
        container: webmMetadata.container,
        track: webmMetadata.tracks?.[0]?.codec,
        videoCodec: webmMetadata.videoCodec,
        cues: webmMetadata.keyframes?.length ?? 0,
        samples: webmSamples.length,
        decoded: webmFrames.length,
        streamed: { count: streamedWebmFrames, returnValue: streamedWebmResult },
        audioSamples: webmAudioSamples.length,
        audioCodec: webmAudioMetadata.audioCodec,
        audioRate: webmAudioMetadata.tracks?.[0]?.sampleRate,
        audioChannels: webmAudioMetadata.tracks?.[0]?.numberOfChannels,
        decodedAudio: webmAudioFrames,
        decodedDimensions: webmDecodedDimensions,
        firstSample: webmSamples[0] ? {
          keyframe: webmSamples[0].keyframe,
          size: webmSamples[0].size,
          timestamp: webmSamples[0].timestamp,
        } : null,
      },
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
  assert.equal(result.webm.container, 'webm');
  assert.equal(result.webm.track, 'vp09.00.10.08');
  assert.equal(result.webm.videoCodec, 'vp09.00.10.08');
  assert.ok(result.webm.cues > 0);
  assert.ok(result.webm.samples > 0);
  assert.equal(result.webm.decoded, 3);
  assert.equal(result.webm.streamed.count, 3);
  assert.equal(result.webm.streamed.returnValue, null);
  assert.ok(result.webm.audioSamples > 0);
  assert.equal(result.webm.audioCodec, 'opus');
  assert.equal(result.webm.audioRate, 48000);
  assert.equal(result.webm.audioChannels, 1);
  assert.ok(result.webm.decodedAudio.length > 0);
  assert.ok(result.webm.decodedAudio.every((frames) => frames > 0));
  assert.deepEqual(result.webm.decodedDimensions[0], [160, 90]);
  assert.ok(result.webm.firstSample.size > 0);
  assert.ok(result.videoDecodeMs >= 0 && result.audioDecodeMs >= 0);
  console.log(JSON.stringify({ status: 'ok', ...result }));
} finally {
  await browser.close();
  await rm(audioFixturePath, { force: true });
  await rm(webmFixturePath, { force: true });
  await rm(webmAudioFixturePath, { force: true });
}
