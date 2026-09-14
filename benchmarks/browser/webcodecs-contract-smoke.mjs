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
const webmVp8FixturePath = `/tmp/dioxuscut-webcodecs-vp8-${process.pid}.webm`;
const webmAv1FixturePath = `/tmp/dioxuscut-webcodecs-av1-${process.pid}.webm`;
const webmAudioFixturePath = `/tmp/dioxuscut-webcodecs-audio-${process.pid}.webm`;
const webmAvFixturePath = `/tmp/dioxuscut-webcodecs-av-${process.pid}.webm`;
await run(process.env.FFMPEG_PATH ?? 'ffmpeg', [
  '-hide_banner', '-loglevel', 'error', '-f', 'lavfi', '-i', 'sine=frequency=440:duration=1',
  '-c:a', 'aac', '-b:a', '96k', '-ar', '48000', '-ac', '1', '-f', 'ipod', '-y', audioFixturePath,
]);
await run(process.env.FFMPEG_PATH ?? 'ffmpeg', [
  '-hide_banner', '-loglevel', 'error', '-f', 'lavfi', '-i', 'testsrc2=size=160x90:rate=24:duration=1',
  '-c:v', 'libvpx-vp9', '-crf', '35', '-b:v', '0', '-an', '-y', webmFixturePath,
]);
await run(process.env.FFMPEG_PATH ?? 'ffmpeg', [
  '-hide_banner', '-loglevel', 'error', '-f', 'lavfi', '-i', 'testsrc2=size=160x90:rate=24:duration=1',
  '-c:v', 'libvpx', '-crf', '35', '-b:v', '0', '-an', '-y', webmVp8FixturePath,
]);
await run(process.env.FFMPEG_PATH ?? 'ffmpeg', [
  '-hide_banner', '-loglevel', 'error', '-f', 'lavfi', '-i', 'testsrc2=size=160x90:rate=24:duration=1',
  '-c:v', 'libsvtav1', '-crf', '45', '-preset', '12', '-an', '-y', webmAv1FixturePath,
]);
await run(process.env.FFMPEG_PATH ?? 'ffmpeg', [
  '-hide_banner', '-loglevel', 'error', '-f', 'lavfi', '-i', 'sine=frequency=440:duration=1',
  '-c:a', 'libopus', '-b:a', '64k', '-ar', '48000', '-ac', '1', '-vn', '-y', webmAudioFixturePath,
]);
await run(process.env.FFMPEG_PATH ?? 'ffmpeg', [
  '-hide_banner', '-loglevel', 'error', '-f', 'lavfi', '-i', 'testsrc2=size=160x90:rate=24:duration=1',
  '-f', 'lavfi', '-i', 'sine=frequency=440:duration=1', '-map', '0:v', '-map', '1:a',
  '-c:v', 'libvpx-vp9', '-crf', '35', '-b:v', '0', '-c:a', 'libopus', '-b:a', '64k', '-shortest', '-y', webmAvFixturePath,
]);
const browser = await chromium.launch({ executablePath, headless: true });
try {
  const page = await browser.newPage();
  const fixture = await readFile(new URL('../../assets/showcase.mp4', import.meta.url));
  const audioFixture = await readFile(audioFixturePath);
  const webmFixture = await readFile(webmFixturePath);
  const webmVp8Fixture = await readFile(webmVp8FixturePath);
  const webmAv1Fixture = await readFile(webmAv1FixturePath);
  const webmAudioFixture = await readFile(webmAudioFixturePath);
  const webmAvFixture = await readFile(webmAvFixturePath);
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
  let rejectedWebmRange = false;
  await page.route('**/assets/webm-416-fixture.webm', (route) => {
    const range = route.request().headers().range;
    if (range && !rejectedWebmRange && !/^bytes=0-3$/.test(range)) {
      rejectedWebmRange = true;
      return route.fulfill({ status: 416, contentType: 'text/plain', body: 'range not satisfiable' });
    }
    return route.fulfill({ status: 200, contentType: 'video/webm', body: webmFixture });
  });
  await page.route('**/assets/webm-vp8-fixture.webm', (route) => route.fulfill({
    status: 200, contentType: 'video/webm', body: webmVp8Fixture,
  }));
  await page.route('**/assets/webm-av1-fixture.webm', (route) => route.fulfill({
    status: 200, contentType: 'video/webm', body: webmAv1Fixture,
  }));
  await page.route('**/assets/webm-audio-fixture.webm', (route) => route.fulfill({
    status: 200, contentType: 'audio/webm', body: webmAudioFixture,
  }));
  await page.route('**/assets/webm-av-fixture.webm', (route) => route.fulfill({
    status: 200, contentType: 'video/webm', body: webmAvFixture,
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
    const webmEvents = [];
    const webmMetadata = await window.dioxuscut.parseMedia({
      src: '/assets/webm-fixture.webm',
      onVideoCodec: (value) => webmEvents.push(['videoCodec', value]),
      onKeyframes: (value) => webmEvents.push(['keyframes', value[0]?.positionInBytes]),
    });
    const webmSamples = await window.dioxuscut.readWebmSamples('/assets/webm-fixture.webm', 1);
    const webm416Samples = await window.dioxuscut.readWebmSamples('/assets/webm-416-fixture.webm', 1, { maxSamples: 1 });
    const webmChunks = webmSamples.slice(0, 2).map((sample) =>
      window.dioxuscut.createWebmEncodedVideoChunk(sample, 1 / 24));
    const webmFrames = await window.dioxuscut.decodeWebmVideo('/assets/webm-fixture.webm', {
      trackNumber: 1, maxSamples: 3, codec: 'vp09.00.10.08',
    });
    const webmDecodedDimensions = webmFrames.map(({ displayWidth, displayHeight }) => [displayWidth, displayHeight]);
    const webmRgba = await window.dioxuscut.videoFrameToRgba(webmFrames[0]);
    if (webmRgba.rgba.byteLength !== webmRgba.width * webmRgba.height * 4
      || webmRgba.width !== 160 || webmRgba.height !== 90) {
      throw new Error(`VideoFrame RGBA transport contract failed: ${webmRgba.width}x${webmRgba.height}/${webmRgba.rgba.byteLength}`);
    }
    for (const frame of webmFrames) frame.close();
    const webmAudioSamples = await window.dioxuscut.readWebmSamples('/assets/webm-audio-fixture.webm', 1);
    const webmVp8Metadata = await window.dioxuscut.parseMedia({ src: '/assets/webm-vp8-fixture.webm' });
    const webmVp8Frames = await window.dioxuscut.decodeWebmVideo('/assets/webm-vp8-fixture.webm', { maxSamples: 1 });
    const webmVp8Dimensions = webmVp8Frames.map(({ displayWidth, displayHeight }) => [displayWidth, displayHeight]);
    for (const frame of webmVp8Frames) frame.close();
    const webmAv1Metadata = await window.dioxuscut.parseMedia({ src: '/assets/webm-av1-fixture.webm' });
    const webmAv1Frames = await window.dioxuscut.decodeWebmVideo('/assets/webm-av1-fixture.webm', { maxSamples: 1 });
    const webmAv1Dimensions = webmAv1Frames.map(({ displayWidth, displayHeight }) => [displayWidth, displayHeight]);
    for (const frame of webmAv1Frames) frame.close();
    const webmAudioMetadata = await window.dioxuscut.parseMedia({ src: '/assets/webm-audio-fixture.webm' });
    const webmAvMetadata = await window.dioxuscut.parseMedia({ src: '/assets/webm-av-fixture.webm' });
    const webmAvVideo = await window.dioxuscut.decodeWebmVideo('/assets/webm-av-fixture.webm', { maxSamples: 1 });
    for (const frame of webmAvVideo) frame.close();
    const webmAvAudio = await window.dioxuscut.decodeWebmAudio('/assets/webm-av-fixture.webm', { maxSamples: 1 });
    const webmAvAudioFrames = webmAvAudio.map(({ numberOfFrames }) => numberOfFrames);
    for (const audio of webmAvAudio) audio.close();
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
    const thirtyFrameDecodeStarted = performance.now();
    const thirtyFrameVideo = await window.dioxuscut.decodeIsoBmffVideo('/assets/showcase.mp4', {
      trackIndex: 0, startSample: 0, endSample: 30, maxSamples: 30,
      codec: codecConfig.codec, description: codecConfig.description, options: { parsed },
    });
    const thirtyFrameDecodeMs = performance.now() - thirtyFrameDecodeStarted;
    let webgpuVideo = null;
    if (navigator.gpu) {
      const adapter = await navigator.gpu.requestAdapter();
      if (adapter) {
        const device = await adapter.requestDevice();
        const width = thirtyFrameVideo[0].displayWidth;
        const height = thirtyFrameVideo[0].displayHeight;
        const texture = device.createTexture({
          size: { width, height, depthOrArrayLayers: 1 },
          format: 'rgba8unorm',
          usage: GPUTextureUsage.COPY_DST | GPUTextureUsage.COPY_SRC | GPUTextureUsage.TEXTURE_BINDING,
        });
        const output = device.createTexture({
          size: { width, height, depthOrArrayLayers: 1 },
          format: 'rgba8unorm',
          usage: GPUTextureUsage.RENDER_ATTACHMENT | GPUTextureUsage.COPY_SRC,
        });
        const shader = device.createShaderModule({ code: `
          @group(0) @binding(0) var video_texture: texture_2d<f32>;
          @group(0) @binding(1) var video_sampler: sampler;
          struct VertexOutput { @builtin(position) position: vec4<f32>, @location(0) uv: vec2<f32> };
          @vertex fn vs(@builtin(vertex_index) index: u32) -> VertexOutput {
            var positions = array<vec2<f32>, 3>(
              vec2<f32>(-1.0, -1.0), vec2<f32>(3.0, -1.0), vec2<f32>(-1.0, 3.0));
            var uvs = array<vec2<f32>, 3>(
              vec2<f32>(0.0, 1.0), vec2<f32>(2.0, 1.0), vec2<f32>(0.0, -1.0));
            var out: VertexOutput;
            out.position = vec4<f32>(positions[index], 0.0, 1.0);
            out.uv = uvs[index];
            return out;
          }
          @fragment fn fs(in: VertexOutput) -> @location(0) vec4<f32> {
            return textureSample(video_texture, video_sampler, in.uv);
          }
        ` });
        const bindGroupLayout = device.createBindGroupLayout({ entries: [
          { binding: 0, visibility: GPUShaderStage.FRAGMENT, texture: {} },
          { binding: 1, visibility: GPUShaderStage.FRAGMENT, sampler: {} },
        ] });
        const pipeline = device.createRenderPipeline({
          layout: device.createPipelineLayout({ bindGroupLayouts: [bindGroupLayout] }),
          vertex: { module: shader, entryPoint: 'vs' },
          fragment: { module: shader, entryPoint: 'fs', targets: [{ format: 'rgba8unorm' }] },
          primitive: { topology: 'triangle-list' },
        });
        const sampler = device.createSampler({ magFilter: 'linear', minFilter: 'linear' });
        const bindGroup = device.createBindGroup({
          layout: bindGroupLayout,
          entries: [
            { binding: 0, resource: texture.createView() },
            { binding: 1, resource: sampler },
          ],
        });
        const bytesPerRow = width * 4;
        const readback = device.createBuffer({
          size: bytesPerRow * height,
          usage: GPUBufferUsage.COPY_DST | GPUBufferUsage.MAP_READ,
        });
        const gpuStarted = performance.now();
        for (const frame of thirtyFrameVideo) {
          device.queue.copyExternalImageToTexture(
            { source: frame },
            { texture },
            { width, height, depthOrArrayLayers: 1 },
          );
          const encoder = device.createCommandEncoder();
          encoder.copyTextureToBuffer(
            { texture },
            { buffer: readback, bytesPerRow, rowsPerImage: height },
            { width, height, depthOrArrayLayers: 1 },
          );
          device.queue.submit([encoder.finish()]);
          await device.queue.onSubmittedWorkDone();
          await readback.mapAsync(GPUMapMode.READ);
          readback.getMappedRange().slice(0, 4);
          readback.unmap();
        }
        const compositeStarted = performance.now();
        for (const frame of thirtyFrameVideo) {
          device.queue.copyExternalImageToTexture(
            { source: frame },
            { texture },
            { width, height, depthOrArrayLayers: 1 },
          );
          const encoder = device.createCommandEncoder();
          const pass = encoder.beginRenderPass({
            colorAttachments: [{
              view: output.createView(),
              clearValue: { r: 0, g: 0, b: 0, a: 1 },
              loadOp: 'clear',
              storeOp: 'store',
            }],
          });
          pass.setPipeline(pipeline);
          pass.setBindGroup(0, bindGroup);
          pass.draw(3);
          pass.end();
          encoder.copyTextureToBuffer(
            { texture: output },
            { buffer: readback, bytesPerRow, rowsPerImage: height },
            { width, height, depthOrArrayLayers: 1 },
          );
          device.queue.submit([encoder.finish()]);
          await device.queue.onSubmittedWorkDone();
          await readback.mapAsync(GPUMapMode.READ);
          readback.getMappedRange().slice(0, 4);
          readback.unmap();
        }
        webgpuVideo = {
          frames: thirtyFrameVideo.length,
          uploadReadbackMs: performance.now() - gpuStarted,
          uploadCompositeReadbackMs: performance.now() - compositeStarted,
          bytes: bytesPerRow * height * thirtyFrameVideo.length,
        };
        readback.destroy();
        output.destroy();
        texture.destroy();
        device.destroy();
      }
    }
    const thirtyFrameRgbaStarted = performance.now();
    let thirtyFrameRgbaBytes = 0;
    for (const frame of thirtyFrameVideo) {
      const rgba = await window.dioxuscut.videoFrameToRgba(frame);
      thirtyFrameRgbaBytes += rgba.rgba.byteLength;
      frame.close();
    }
    const thirtyFrameRgbaMs = performance.now() - thirtyFrameRgbaStarted;
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
      thirtyFrame: {
        decoded: thirtyFrameVideo.length,
        decodeMs: thirtyFrameDecodeMs,
        rgbaMs: thirtyFrameRgbaMs,
        rgbaBytes: thirtyFrameRgbaBytes,
        webgpu: webgpuVideo,
      },
      decodedAudio,
      audioDecodeMs,
      streamedVideo: { count: streamedVideo.length, returnValue: streamedVideoResult },
      streamedAudio: { count: streamedAudio.length, returnValue: streamedAudioResult },
      webm: {
        container: webmMetadata.container,
        track: webmMetadata.tracks?.[0]?.codec,
        videoCodec: webmMetadata.videoCodec,
        keyframeCallback: webmEvents,
        cues: webmMetadata.keyframes?.length ?? 0,
        samples: webmSamples.length,
        rangeFallbackSamples: webm416Samples.length,
        decoded: webmFrames.length,
        rgbaTransport: { width: webmRgba.width, height: webmRgba.height, bytes: webmRgba.rgba.byteLength },
        streamed: { count: streamedWebmFrames, returnValue: streamedWebmResult },
        audioSamples: webmAudioSamples.length,
        vp8: { codec: webmVp8Metadata.videoCodec, decoded: webmVp8Dimensions },
        av1: { codec: webmAv1Metadata.videoCodec, decoded: webmAv1Dimensions },
        audioCodec: webmAudioMetadata.audioCodec,
        audioRate: webmAudioMetadata.tracks?.[0]?.sampleRate,
        audioChannels: webmAudioMetadata.tracks?.[0]?.numberOfChannels,
        combined: {
          tracks: webmAvMetadata.tracks?.length,
          videoTrack: webmAvMetadata.tracks?.find((track) => track.type === 'video')?.trackNumber,
          audioTrack: webmAvMetadata.tracks?.find((track) => track.type === 'audio')?.trackNumber,
          audioFrames: webmAvAudioFrames,
        },
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
  assert.equal(result.thirtyFrame.decoded, 30);
  assert.ok(result.thirtyFrame.decodeMs > 0);
  assert.ok(result.thirtyFrame.rgbaMs > 0);
  assert.equal(result.thirtyFrame.rgbaBytes, 30 * 1920 * 1080 * 4);
  assert.equal(result.streamedVideo.count, 3);
  assert.equal(result.streamedVideo.returnValue, null);
  assert.ok(result.streamedAudio.count > 0);
  assert.equal(result.streamedAudio.returnValue, null);
  assert.equal(result.webm.container, 'webm');
  assert.equal(result.webm.track, 'vp09.00.10.08');
  assert.equal(result.webm.videoCodec, 'vp09.00.10.08');
  assert.ok(result.webm.keyframeCallback.some(([name, position]) => name === 'keyframes' && position > 0));
  assert.ok(result.webm.cues > 0);
  assert.ok(result.webm.samples > 0);
  assert.equal(result.webm.rangeFallbackSamples, 1);
  assert.equal(result.webm.decoded, 3);
  assert.equal(result.webm.streamed.count, 3);
  assert.equal(result.webm.streamed.returnValue, null);
  assert.ok(result.webm.audioSamples > 0);
  assert.equal(result.webm.vp8.codec, 'vp8');
  assert.deepEqual(result.webm.vp8.decoded[0], [160, 90]);
  assert.equal(result.webm.av1.codec, 'av01.0.08M.08');
  assert.deepEqual(result.webm.av1.decoded[0], [160, 90]);
  assert.equal(result.webm.audioCodec, 'opus');
  assert.equal(result.webm.audioRate, 48000);
  assert.equal(result.webm.audioChannels, 1);
  assert.ok(result.webm.decodedAudio.length > 0);
  assert.ok(result.webm.decodedAudio.every((frames) => frames > 0));
  assert.equal(result.webm.combined.tracks, 2);
  assert.equal(result.webm.combined.videoTrack, 1);
  assert.equal(result.webm.combined.audioTrack, 2);
  assert.ok(result.webm.combined.audioFrames[0] > 0);
  assert.deepEqual(result.webm.decodedDimensions[0], [160, 90]);
  assert.ok(result.webm.firstSample.size > 0);
  assert.ok(result.videoDecodeMs >= 0 && result.audioDecodeMs >= 0);
  console.log(JSON.stringify({ status: 'ok', ...result }));
} finally {
  await browser.close();
  await rm(audioFixturePath, { force: true });
  await rm(webmFixturePath, { force: true });
  await rm(webmVp8FixturePath, { force: true });
  await rm(webmAv1FixturePath, { force: true });
  await rm(webmAudioFixturePath, { force: true });
  await rm(webmAvFixturePath, { force: true });
}
