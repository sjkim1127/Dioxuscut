import { invoke } from '@tauri-apps/api/core';
import { join, tempDir } from '@tauri-apps/api/path';
import { open, save } from '@tauri-apps/plugin-dialog';
import './style.css';

const app = document.querySelector('#app');
app.innerHTML = `
  <main class="studio">
    <header><h1>Dioxuscut Studio</h1><span id="backend">connecting…</span></header>
    <section class="toolbar">
      <input id="project-path" value="/tmp/dioxuscut-project.json" aria-label="project path" />
      <button id="load-project">Open project</button>
      <button id="save-project">Save project</button>
      <button id="submit-render">Queue render</button>
      <button id="cancel-render" disabled>Cancel</button>
      <span id="message"></span>
    </section>
    <section class="timeline">
      <button id="play-toggle">Pause</button>
      <input id="timeline-slider" type="range" min="0" max="149" value="0" aria-label="timeline frame" />
      <span id="timeline-frame">0 / 149</span>
    </section>
    <section class="preview"><canvas id="preview-canvas"></canvas></section>
    <section class="job-queue"><h2>Render queue</h2><div id="job-list">No jobs</div></section>
    <footer><span id="frame">frame 0</span><span id="protocol">worker protocol…</span><span id="job">job store…</span></footer>
  </main>`;

const canvas = document.querySelector('#preview-canvas');
let THREE;
let renderer;
let scene;
let camera;
let cube;
let threeReady;
async function ensureThree() {
  if (threeReady) return threeReady;
  threeReady = import('three').then((module) => {
    THREE = module;
    renderer = new THREE.WebGLRenderer({ canvas, antialias: true, alpha: false });
    renderer.setPixelRatio(Math.min(window.devicePixelRatio, 2));
    renderer.setClearColor(0x0b1020);
    scene = new THREE.Scene();
    camera = new THREE.PerspectiveCamera(45, 1, 0.1, 100);
    camera.position.z = 3;
    scene.add(new THREE.HemisphereLight(0x9bbcff, 0x182033, 2));
    cube = new THREE.Mesh(
      new THREE.BoxGeometry(1, 1, 1),
      new THREE.MeshStandardMaterial({ color: 0x6c63ff, roughness: 0.28, metalness: 0.35 }),
    );
    scene.add(cube);
  });
  return threeReady;
}

// Browser compositions can replace the demo scene without changing the Rust
// protocol. Adapters may register Three.js, R3F, or another WebGL renderer.
const compositions = new Map();
const threeCompositions = new Map();
const preloadedAssets = new Map();
const preloadedSources = new Map();
const imageDimensionsCache = new Map();
const videoMetadataCache = new Map();
const audioDurationCache = new Map();
const audioDataCache = new Map();
const videoTextureCache = new Map();
const renderGates = new Map();
let lottieAdapter = null;
const lottieInstances = new WeakMap();
let lottieModulePromise;
let nextRenderGate = 1;
let renderGateError = null;

function delayRender(reason = 'render gate') {
  const handle = nextRenderGate++;
  let resolve;
  let reject;
  const promise = new Promise((resolvePromise, rejectPromise) => {
    resolve = resolvePromise;
    reject = rejectPromise;
  });
  renderGates.set(handle, { promise, reason, resolve, reject });
  return handle;
}

function continueRender(handle) {
  const gate = renderGates.get(handle);
  if (!gate) return;
  renderGates.delete(handle);
  gate.resolve();
}

function cancelRender(handle, reason = 'render cancelled') {
  if (!renderGates.has(handle)) return;
  renderGates.delete(handle);
  renderGateError = new Error(String(reason));
}

// Hook-shaped aliases for compositions ported from Remotion. The worker is
// not React-driven, so these return the stable render-gate protocol helpers.
export function useDelayRender() {
  return { delayRender, continueRender, cancelRender };
}

// Minimal browser equivalent of Remotion's useBufferState. Media adapters can
// pause frame capture while a seek or decoder warm-up is pending, then release
// exactly the handle they acquired.
export function useBufferState() {
  return {
    delayPlayback() {
      const handle = delayRender('media buffering');
      return { unblock: () => continueRender(handle) };
    },
  };
}

// Browser pixel density for canvas/WebGL adapters. Headless workers use their
// configured device scale factor, keeping captures deterministic.
export function usePixelDensity() {
  return window.devicePixelRatio || 1;
}

async function waitForRenderGates() {
  if (renderGateError) throw renderGateError;
  while (renderGates.size > 0) {
    await Promise.all([...renderGates.values()].map((gate) => gate.promise));
    if (renderGateError) throw renderGateError;
  }
}
async function preloadAssets(assets = []) {
  await Promise.all(assets.map(async (source) => {
    if (preloadedAssets.has(source)) return preloadedAssets.get(source);
    const extension = source.split(/[?#]/, 1)[0].split('.').pop()?.toLowerCase();
    let task;
    if (['ttf', 'otf', 'woff', 'woff2'].includes(extension)) {
      task = new FontFace(`dioxuscut-${preloadedAssets.size}`, `url(${source})`).load();
      task = task.then((font) => { document.fonts.add(font); return font; });
    } else if (extension === 'json') {
      task = fetch(source).then((response) => {
        if (!response.ok) throw new Error(`failed to preload asset: ${source}`);
        return response.json();
      });
    } else if (['mp4', 'webm', 'mov', 'm4v'].includes(extension)) {
      task = new Promise((resolve, reject) => {
        const video = document.createElement('video');
        video.preload = 'metadata';
        video.onloadedmetadata = () => resolve(video);
        video.onerror = () => reject(new Error(`failed to preload asset: ${source}`));
        video.src = source;
      });
    } else {
      task = new Promise((resolve, reject) => {
        const image = new Image();
        image.onload = () => image.decode().then(() => resolve(image), () => resolve(image));
        image.onerror = () => reject(new Error(`failed to preload asset: ${source}`));
        image.src = source;
      });
    }
    task = task.catch((error) => {
      preloadedAssets.delete(source);
      throw error;
    });
    preloadedAssets.set(source, task);
    return task;
  }));
}

export function registerComposition(id, render) {
  if (typeof id !== 'string' || !id || typeof render !== 'function') {
    throw new TypeError('registerComposition expects a non-empty id and render function');
  }
  compositions.set(id, render);
}

/**
 * Register a reusable Three.js composition with an explicit scene lifecycle.
 *
 * `setup` runs once per browser worker and may return `{scene, camera}` (or
 * any additional application state). `render` runs for every requested frame.
 * Keeping this contract outside the frame protocol lets the same scene module
 * run in Studio, the headless worker, and a future R3F adapter.
 */
export function registerThreeComposition(id, { setup, render, dispose } = {}) {
  if (typeof id !== 'string' || !id || typeof setup !== 'function' || typeof render !== 'function') {
    throw new TypeError('registerThreeComposition expects id, setup(), and render()');
  }
  const previous = threeCompositions.get(id);
  previous?.dispose?.();
  const entry = { setup, render, dispose, instance: null };
  threeCompositions.set(id, entry);
  registerComposition(id, async (context) => {
    if (!entry.instance) {
      await ensureThree();
      entry.instance = await setup({ THREE, renderer, canvas, ...context });
    }
    return render({
      THREE,
      renderer,
      canvas,
      ...context,
      ...(entry.instance && typeof entry.instance === 'object' ? entry.instance : {}),
    });
  });
}

export function unregisterThreeComposition(id) {
  const entry = threeCompositions.get(id);
  if (!entry) return false;
  entry.dispose?.(entry.instance);
  entry.instance?.renderer?.dispose?.();
  entry.instance?.scene?.traverse?.((object) => {
    object.geometry?.dispose?.();
    for (const material of Array.isArray(object.material) ? object.material : [object.material]) {
      material?.dispose?.();
    }
  });
  threeCompositions.delete(id);
  compositions.delete(id);
  return true;
}

// Browser equivalent of @remotion/media-utils/getImageDimensions. Dimensions
// are cached independently from decoded assets so layout probes do not force a
// second request or decode in a composition.
export async function getImageDimensions(source) {
  if (typeof source !== 'string' || !source) throw new TypeError('getImageDimensions expects a source URL');
  if (imageDimensionsCache.has(source)) return imageDimensionsCache.get(source);
  const dimensions = new Promise((resolve, reject) => {
    const image = new Image();
    image.onload = () => resolve({ width: image.naturalWidth, height: image.naturalHeight });
    image.onerror = () => reject(new Error(`failed to load image dimensions: ${source}`));
    image.src = source;
  }).catch((error) => {
    imageDimensionsCache.delete(source);
    throw error;
  });
  imageDimensionsCache.set(source, dimensions);
  return dimensions;
}

// Browser equivalent of @remotion/media-utils/getVideoMetadata.
export async function getVideoMetadata(source) {
  if (typeof source !== 'string' || !source) throw new TypeError('getVideoMetadata expects a source URL');
  if (videoMetadataCache.has(source)) return videoMetadataCache.get(source);
  const metadata = new Promise((resolve, reject) => {
    const video = document.createElement('video');
    const cleanup = () => { video.removeAttribute('src'); video.load(); };
    video.preload = 'metadata';
    video.onloadedmetadata = () => {
      if (!video.videoWidth || !video.videoHeight || !Number.isFinite(video.duration)) {
        reject(new Error(`unable to determine video metadata: ${source}`));
        cleanup();
        return;
      }
      resolve({
        durationInSeconds: video.duration,
        width: video.videoWidth,
        height: video.videoHeight,
        aspectRatio: video.videoWidth / video.videoHeight,
        isRemote: /^https?:\/\//i.test(source),
      });
      cleanup();
    };
    video.onerror = () => { reject(new Error(`failed to load video metadata: ${source}`)); cleanup(); };
    video.src = source;
  }).catch((error) => {
    videoMetadataCache.delete(source);
    throw error;
  });
  videoMetadataCache.set(source, metadata);
  return metadata;
}

// Metadata-first browser facade corresponding to @remotion/media-parser's
// parseMedia(). It reuses the existing cached probes and intentionally does
// not download sample payloads; callers that need samples use getAudioData or
// the video texture adapters.
export async function parseMedia({
  src,
  fields,
  onDimensions,
  onDurationInSeconds,
  onParseProgress,
} = {}) {
  if (typeof src !== 'string' || !src) throw new TypeError('parseMedia expects {src}');
  for (const [name, callback] of Object.entries({ onDimensions, onDurationInSeconds, onParseProgress })) {
    if (callback !== undefined && typeof callback !== 'function') {
      throw new TypeError(`parseMedia expects ${name} to be a function`);
    }
  }
  // This metadata facade does not yet expose container byte counts. Report the
  // same progress shape as @remotion/media-parser with an unknown total.
  await onParseProgress?.({ bytes: 0, percentage: 0, totalBytes: null });
  const selectFields = (result) => !Array.isArray(fields) || fields.length === 0
    ? result
    : Object.fromEntries(fields.filter((field) => field in result).map((field) => [field, result[field]]));
  const wav = await parseWavMetadata(src).catch(() => null);
  if (wav) {
    await onDimensions?.(null);
    await onDurationInSeconds?.(wav.durationInSeconds);
    await onParseProgress?.({ bytes: 0, percentage: 1, totalBytes: null });
    return selectFields(wav);
  }
  const [video, image, audioDuration] = await Promise.all([
    getVideoMetadata(src).catch(() => null),
    getImageDimensions(src).catch(() => null),
    getAudioDurationInSeconds(src).catch(() => null),
  ]);
  if (!video && !image && audioDuration === null) {
    throw new Error(`unable to parse media metadata: ${src}`);
  }
  const dimensions = video ? { width: video.width, height: video.height } : image;
  const durationInSeconds = video?.durationInSeconds ?? audioDuration ?? 0;
  await onDimensions?.(dimensions);
  await onDurationInSeconds?.(durationInSeconds);
  await onParseProgress?.({ bytes: 0, percentage: 1, totalBytes: null });
  const result = {
    durationInSeconds,
    dimensions,
    videoTracks: video ? [{ width: video.width, height: video.height, aspectRatio: video.aspectRatio }] : [],
    audioTracks: audioDuration !== null ? [{ durationInSeconds: audioDuration }] : [],
    isRemote: /^https?:\/\//i.test(src),
  };
  return selectFields(result);
}

// Browser counterpart of the native bounded range reader used by the
// parseMedia foundation. `endExclusive` follows the same half-open contract.
export async function readMediaRange(source, start, endExclusive, { requestInit } = {}) {
  if (typeof source !== 'string' || !source) throw new TypeError('readMediaRange expects a source URL');
  if (!Number.isInteger(start) || !Number.isInteger(endExclusive) || start < 0 || endExclusive < start) {
    throw new RangeError('readMediaRange expects a non-negative half-open range');
  }
  const length = endExclusive - start;
  if (length > 16 * 1024 * 1024) throw new RangeError('media range exceeds 16 MiB');
  if (length === 0) return new Uint8Array();
  const headers = new Headers(requestInit?.headers);
  headers.set('Range', `bytes=${start}-${endExclusive - 1}`);
  const response = await fetch(source, { ...requestInit, headers });
  if (!response.ok) throw new Error(`media range request failed: ${response.status}`);
  const bytes = new Uint8Array(await response.arrayBuffer());
  if (bytes.length > length && response.status !== 206) {
    throw new Error('media range response exceeded requested length');
  }
  if (response.status === 206 && bytes.length !== length) {
    throw new Error(`media range response length ${bytes.length} did not match requested length ${length}`);
  }
  return bytes;
}

function ascii(bytes, offset, length) {
  return String.fromCharCode(...bytes.subarray(offset, offset + length));
}

function uint32le(bytes, offset) {
  return (bytes[offset] | (bytes[offset + 1] << 8) | (bytes[offset + 2] << 16) | (bytes[offset + 3] << 24)) >>> 0;
}

// Incremental WAV probe: only RIFF headers and chunk headers are fetched.
// This mirrors the bounded-reader design of @remotion/media-parser without
// downloading PCM payloads merely to answer metadata queries.
export async function parseWavMetadata(source, { requestInit } = {}) {
  const header = await readMediaRange(source, 0, 12, { requestInit });
  if (header.length < 12 || ascii(header, 0, 4) !== 'RIFF' || ascii(header, 8, 4) !== 'WAVE') return null;
  let offset = 12;
  let format = null;
  let dataBytes = null;
  for (let chunk = 0; chunk < 4096; chunk += 1) {
    const chunkHeader = await readMediaRange(source, offset, offset + 8, { requestInit });
    if (chunkHeader.length < 8) break;
    const id = ascii(chunkHeader, 0, 4);
    const size = uint32le(chunkHeader, 4);
    if (!Number.isSafeInteger(size) || size < 0) break;
    if (id === 'fmt ' && size >= 16) {
      const bytes = await readMediaRange(source, offset + 8, offset + 24, { requestInit });
      format = {
        audioFormat: bytes[0] | (bytes[1] << 8),
        channels: bytes[2] | (bytes[3] << 8),
        sampleRate: uint32le(bytes, 4) >>> 0,
        blockAlign: bytes[12] | (bytes[13] << 8),
      };
    } else if (id === 'data') {
      dataBytes = size;
    }
    offset += 8 + size + (size & 1);
    if (format && dataBytes !== null) break;
  }
  if (!format || !format.channels || !format.sampleRate || !format.blockAlign || dataBytes === null) return null;
  const durationInSeconds = dataBytes / (format.sampleRate * format.blockAlign);
  return {
    durationInSeconds,
    audioTracks: [{ channels: format.channels, sampleRate: format.sampleRate }],
    dimensions: null,
    videoTracks: [],
    isRemote: /^https?:\/\//i.test(source),
    audioFormat: format.audioFormat,
  };
}

// Browser equivalent of @remotion/media-utils/getAudioDurationInSeconds.
export async function getAudioDurationInSeconds(source) {
  if (typeof source !== 'string' || !source) throw new TypeError('getAudioDurationInSeconds expects a source URL');
  if (audioDurationCache.has(source)) return audioDurationCache.get(source);
  const duration = new Promise((resolve, reject) => {
    const audio = document.createElement('audio');
    const cleanup = () => { audio.removeAttribute('src'); audio.load(); };
    audio.preload = 'metadata';
    audio.onloadedmetadata = () => {
      if (!Number.isFinite(audio.duration)) {
        reject(new Error(`unable to determine audio duration: ${source}`));
        cleanup();
        return;
      }
      resolve(audio.duration);
      cleanup();
    };
    audio.onerror = () => { reject(new Error(`failed to load audio metadata: ${source}`)); cleanup(); };
    audio.src = source;
  }).catch((error) => {
    audioDurationCache.delete(source);
    throw error;
  });
  audioDurationCache.set(source, duration);
  return duration;
}

export const getAudioDuration = getAudioDurationInSeconds;

// Browser counterpart of @remotion/media-utils/getAudioData. Decode once per
// source and expose channel-major PCM data so audio visualizers can share the
// same frame-driven contract as native compositions.
export async function getAudioData(source, { sampleRate = 48_000, requestInit } = {}) {
  if (typeof source !== 'string' || !source) throw new TypeError('getAudioData expects a source URL');
  if (!Number.isFinite(sampleRate) || sampleRate <= 0) throw new TypeError('sampleRate must be positive');
  if (audioDataCache.has(source)) return audioDataCache.get(source);
  const task = (async () => {
    const AudioContext = window.AudioContext || window.webkitAudioContext;
    if (!AudioContext) throw new Error('Web Audio API is unavailable');
    const response = await fetch(source, requestInit);
    if (!response.ok) throw new Error(`failed to load audio data: ${source}`);
    const context = new AudioContext({ sampleRate });
    try {
      const buffer = await context.decodeAudioData(await response.arrayBuffer());
      const channelWaveforms = Array.from({ length: buffer.numberOfChannels }, (_, channel) =>
        buffer.getChannelData(channel));
      return {
        // `channelWaveforms` is Remotion's v4 field; `channelData` is kept as
        // the Web Audio-friendly alias used by browser visualizer libraries.
        channelWaveforms,
        channelData: channelWaveforms,
        sampleRate: buffer.sampleRate,
        durationInSeconds: buffer.duration,
        numberOfChannels: buffer.numberOfChannels,
        resultId: source,
        isRemote: /^https?:\/\//i.test(source),
      };
    } finally {
      await context.close();
    }
  })().catch((error) => {
    audioDataCache.delete(source);
    throw error;
  });
  audioDataCache.set(source, task);
  return task;
}

// Hook-shaped alias for browser compositions. Consumers can await the same
// promise from a frame render and use delayRender around it when necessary.
export const useAudioData = getAudioData;

// Remotion-compatible AudioBuffer -> float32 WAV data URL. Keeping this in
// the browser host lets generated compositions feed synthesized audio back
// into Html5Audio or an export request without a server round-trip.
export function audioBufferToDataUrl(buffer) {
  if (!buffer || !Number.isInteger(buffer.numberOfChannels) || buffer.numberOfChannels < 1) {
    throw new TypeError('audioBufferToDataUrl expects an AudioBuffer');
  }
  const channels = Array.from({ length: buffer.numberOfChannels }, (_, channel) => buffer.getChannelData(channel));
  const frames = channels[0].length;
  const interleaved = new Float32Array(frames * buffer.numberOfChannels);
  for (let frame = 0; frame < frames; frame += 1) {
    for (let channel = 0; channel < channels.length; channel += 1) {
      interleaved[frame * channels.length + channel] = channels[channel][frame] ?? 0;
    }
  }
  const bytesPerSample = 4;
  const blockAlign = channels.length * bytesPerSample;
  const output = new ArrayBuffer(44 + interleaved.length * bytesPerSample);
  const view = new DataView(output);
  const writeString = (offset, value) => [...value].forEach((character, index) => view.setUint8(offset + index, character.charCodeAt(0)));
  writeString(0, 'RIFF'); view.setUint32(4, 36 + interleaved.length * bytesPerSample, true);
  writeString(8, 'WAVE'); writeString(12, 'fmt '); view.setUint32(16, 16, true);
  view.setUint16(20, 3, true); view.setUint16(22, channels.length, true);
  view.setUint32(24, buffer.sampleRate, true); view.setUint32(28, buffer.sampleRate * blockAlign, true);
  view.setUint16(32, blockAlign, true); view.setUint16(34, 32, true); writeString(36, 'data');
  view.setUint32(40, interleaved.length * bytesPerSample, true);
  for (let index = 0; index < interleaved.length; index += 1) view.setFloat32(44 + index * 4, interleaved[index], true);
  const bytes = new Uint8Array(output);
  let binary = '';
  for (let offset = 0; offset < bytes.length; offset += 0x8000) {
    binary += String.fromCharCode(...bytes.subarray(offset, Math.min(offset + 0x8000, bytes.length)));
  }
  return `data:audio/wav;base64,${window.btoa(binary)}`;
}

export function prefetch(source, {
  method = 'blob-url', credentials, contentType, onProgress,
} = {}) {
  if (typeof source !== 'string' || !source) throw new TypeError('prefetch expects a source URL');
  const hashIndex = source.indexOf('#');
  const base = hashIndex < 0 ? source : source.slice(0, hashIndex);
  const controller = new AbortController();
  let released = false;
  let objectUrl = null;
  const done = fetch(base, { credentials, signal: controller.signal }).then(async (response) => {
    if (!response.ok) throw new Error(`prefetch failed: ${response.status} ${response.statusText}`);
    if (!response.body) throw new Error('prefetch response has no body');
    const reader = response.body.getReader();
    const chunks = [];
    let received = 0;
    const total = Number(response.headers.get('content-length')) || null;
    while (true) {
      const { done, value } = await reader.read();
      if (done) break;
      chunks.push(value);
      received += value.byteLength;
      onProgress?.({ loadedBytes: received, totalBytes: total });
    }
    const blob = new Blob(chunks, { type: response.headers.get('content-type') || undefined });
    if (released) return base;
    if (method === 'base64') {
      const bytes = new Uint8Array(await blob.arrayBuffer());
      let binary = '';
      for (let offset = 0; offset < bytes.length; offset += 0x8000) {
        binary += String.fromCharCode(...bytes.subarray(offset, Math.min(offset + 0x8000, bytes.length)));
      }
      const type = contentType || blob.type || 'application/octet-stream';
      return `data:${type};base64,${window.btoa(binary)}`;
    }
    objectUrl = URL.createObjectURL(contentType ? new Blob([blob], { type: contentType }) : blob);
    return objectUrl;
  });
  done.then((resolved) => {
    if (!released) preloadedSources.set(base, resolved);
  }).catch(() => undefined);
  return {
    free() {
      released = true;
      controller.abort();
      if (objectUrl) URL.revokeObjectURL(objectUrl);
      preloadedSources.delete(base);
    },
    waitUntilDone: () => done,
  };
}

export function usePreload(source) {
  if (typeof source !== 'string') return source;
  const hashIndex = source.indexOf('#');
  const base = hashIndex < 0 ? source : source.slice(0, hashIndex);
  const suffix = hashIndex < 0 ? '' : source.slice(hashIndex);
  const loaded = preloadedSources.get(base);
  return loaded ? `${loaded}${suffix}` : source;
}

// Platform-neutral counterpart of Remotion's useWindowedAudioData. The host
// does not require React: callers receive the window centered on the requested
// frame plus its timeline offset, while getAudioData() keeps decoding cached.
export async function getWindowedAudioData(source, {
  frame, fps, windowInSeconds, channelIndex = 0,
} = {}) {
  if (!Number.isFinite(frame) || !Number.isFinite(fps) || fps <= 0) {
    throw new TypeError('getWindowedAudioData requires a finite frame and positive fps');
  }
  if (!Number.isFinite(windowInSeconds) || windowInSeconds <= 0) {
    throw new TypeError('windowInSeconds must be positive');
  }
  const audioData = await getAudioData(source);
  if (!Number.isInteger(channelIndex) || channelIndex < 0 || channelIndex >= audioData.numberOfChannels) {
    throw new RangeError(`Invalid channel index ${channelIndex} for ${audioData.numberOfChannels} channels`);
  }
  const currentTime = frame / fps;
  const windowIndex = Math.floor(currentTime / windowInSeconds);
  const startTime = windowIndex * windowInSeconds;
  const startSample = Math.max(0, Math.floor(startTime * audioData.sampleRate));
  const endSample = Math.min(
    audioData.channelWaveforms[channelIndex].length,
    Math.ceil((startTime + windowInSeconds) * audioData.sampleRate),
  );
  const waveform = audioData.channelWaveforms[channelIndex].slice(startSample, endSample);
  return {
    audioData: {
      ...audioData,
      channelWaveforms: [waveform],
      channelData: [waveform],
      numberOfChannels: 1,
      durationInSeconds: waveform.length / audioData.sampleRate,
      resultId: `${audioData.resultId}:window:${channelIndex}:${windowIndex}`,
    },
    dataOffsetInSeconds: startSample / audioData.sampleRate,
  };
}

// In the framework-free host this hook-shaped name is an async adapter rather
// than a React hook; it preserves the vendor import name without requiring a
// React runtime in Three.js compositions.
export const useWindowedAudioData = getWindowedAudioData;

// Lightweight browser equivalent of getWaveformPortion(). It preserves the
// frame/time contract while reducing decoded PCM into visualization bars.
export function getWaveformPortion({
  audioData, startTimeInSeconds, durationInSeconds, numberOfSamples,
  channel = 0, dataOffsetInSeconds = 0, outputRange = 'zero-to-one', normalize = true,
}) {
  const channels = audioData?.channelWaveforms ?? audioData?.channelData;
  if (!channels?.length || numberOfSamples <= 0) return [];
  const waveform = channels[Math.min(channel, channels.length - 1)];
  const start = Math.floor((startTimeInSeconds - dataOffsetInSeconds) * audioData.sampleRate);
  const end = Math.floor((startTimeInSeconds - dataOffsetInSeconds + durationInSeconds) * audioData.sampleRate);
  const padded = new Float32Array(Math.max(0, end - start));
  for (let sample = Math.max(0, start); sample < Math.min(waveform.length, end); sample += 1) {
    padded[sample - start] = waveform[sample];
  }
  const blockSize = Math.floor(padded.length / numberOfSamples);
  if (blockSize === 0) return [];
  const values = Array.from({ length: numberOfSamples }, (_, index) => {
    let sum = 0;
    for (let sample = 0; sample < blockSize; sample += 1) {
      sum += Math.abs(padded[index * blockSize + sample]);
    }
    return sum / blockSize;
  });
  const scale = normalize ? Math.max(...values, 1e-9) : 1;
  return values.map((value, index) => ({
    index,
    amplitude: outputRange === 'minus-one-to-one'
      ? (value / scale) * (index % 2 === 0 ? -1 : 1)
      : value / scale,
  }));
}

export function visualizeAudioWaveform({
  audioData, frame, fps, windowInSeconds, numberOfSamples,
  channel = 0, dataOffsetInSeconds = 0, normalize = false,
}) {
  if (windowInSeconds * audioData.sampleRate < numberOfSamples) {
    throw new TypeError('windowInSeconds must provide at least one audio sample per bar');
  }
  return getWaveformPortion({
    audioData,
    startTimeInSeconds: frame / fps - windowInSeconds / 2,
    durationInSeconds: windowInSeconds,
    numberOfSamples,
    channel,
    dataOffsetInSeconds,
    outputRange: 'minus-one-to-one',
    normalize,
  }).map(({ amplitude }) => amplitude);
}

const visualizeAudioCache = new Map();

function fftMagnitudes(samples) {
  const size = samples.length;
  const real = new Float64Array(samples);
  const imaginary = new Float64Array(size);
  for (let i = 1, j = 0; i < size; i += 1) {
    let bit = size >> 1;
    for (; j & bit; bit >>= 1) j ^= bit;
    j ^= bit;
    if (i < j) {
      const value = real[i]; real[i] = real[j]; real[j] = value;
    }
  }
  for (let length = 2; length <= size; length <<= 1) {
    const angle = -2 * Math.PI / length;
    for (let offset = 0; offset < size; offset += length) {
      for (let i = 0; i < length / 2; i += 1) {
        const phase = angle * i;
        const even = offset + i;
        const odd = even + length / 2;
        const cos = Math.cos(phase);
        const sin = Math.sin(phase);
        const oddReal = real[odd] * cos - imaginary[odd] * sin;
        const oddImaginary = real[odd] * sin + imaginary[odd] * cos;
        real[odd] = real[even] - oddReal;
        imaginary[odd] = imaginary[even] - oddImaginary;
        real[even] += oddReal;
        imaginary[even] += oddImaginary;
      }
    }
  }
  return Array.from({ length: size / 2 }, (_, index) =>
    Math.hypot(real[index], imaginary[index]));
}

function accurateFftMagnitudes(samples) {
  const size = samples.length;
  const complex = accurateFftComplex(samples);
  return Array.from({ length: size / 2 }, (_, index) =>
    Math.hypot(complex[index * 2], complex[index * 2 + 1]));
}

function accurateFftComplex(samples) {
  const size = samples.length;
  if (size === 1) return new Float64Array([samples[0], 0]);
  const evens = new Float64Array(size / 2);
  const odds = new Float64Array(size / 2);
  for (let index = 0; index < size / 2; index += 1) {
    evens[index] = samples[index * 2];
    odds[index] = samples[index * 2 + 1];
  }
  const even = accurateFftComplex(evens);
  const odd = accurateFftComplex(odds);
  const result = new Float64Array(size * 2);
  for (let index = 0; index < size; index += 2) {
    const angle = -Math.PI * index / size;
    const oddReal = odd[index] * Math.cos(angle) - odd[index + 1] * Math.sin(angle);
    const oddImaginary = odd[index] * Math.sin(angle) + odd[index + 1] * Math.cos(angle);
    result[index] = even[index / 2] + oddReal;
    result[index + 1] = even[index / 2 + 1] + oddImaginary;
    result[index + size] = even[index / 2] - oddReal;
    result[index + size + 1] = even[index / 2 + 1] - oddImaginary;
  }
  return result;
}

export function visualizeAudio({
  audioData, frame, fps, numberOfSamples, optimizeFor = 'accuracy',
  dataOffsetInSeconds = 0, smoothing = true,
}) {
  const size = numberOfSamples * 2;
  if (!Number.isInteger(numberOfSamples) || numberOfSamples <= 0 || (size & (size - 1)) !== 0) {
    throw new TypeError(`numberOfSamples must produce a power-of-two FFT size; got ${numberOfSamples}`);
  }
  if (!fps) throw new TypeError('fps is required');
  const waveform = (audioData?.channelWaveforms ?? audioData?.channelData)?.[0];
  if (!waveform || waveform.length < size) throw new TypeError(`Audio data is not big enough to provide ${size} bars.`);
  const start = Math.floor((frame / fps - dataOffsetInSeconds) * audioData.sampleRate);
  const actualStart = Math.max(0, start - size / 2);
  const cacheKey = `${audioData.resultId}:${frame}:${fps}:${numberOfSamples}:${optimizeFor}:${dataOffsetInSeconds}`;
  const compute = () => {
    const samples = new Float64Array(size);
    for (let i = 0; i < size; i += 1) {
      const value = waveform[actualStart + i] ?? 0;
      samples[i] = Math.max(-1, Math.min(1, value)) * 32767;
    }
    const magnitudes = optimizeFor === 'accuracy'
      ? accurateFftMagnitudes(samples)
      : fftMagnitudes(samples);
    let maxMagnitude = 0;
    for (const sample of waveform) maxMagnitude = Math.max(maxMagnitude, Math.abs(sample));
    const maxInt = maxMagnitude * 32767 || 1;
    return magnitudes.map((value) => Math.max(0, Math.min(1, value / (size / 2) / maxInt)));
  };
  const current = visualizeAudioCache.get(cacheKey) ?? compute();
  visualizeAudioCache.set(cacheKey, current);
  if (!smoothing) return current;
  const neighbours = [frame - 1, frame + 1].map((nearbyFrame) => visualizeAudio({
    audioData, frame: nearbyFrame, fps, numberOfSamples, optimizeFor,
    dataOffsetInSeconds, smoothing: false,
  }));
  return current.map((value, index) => (value + neighbours[0][index] + neighbours[1][index]) / 3);
}

// Remotion-compatible Catmull-Rom-style SVG path helper for browser scenes.
export function createSmoothSvgPath({ points = [] } = {}) {
  const line = (a, b) => ({
    length: Math.hypot(b.x - a.x, b.y - a.y),
    angle: Math.atan2(b.y - a.y, b.x - a.x),
  });
  const controlPoint = (current, previous, next, reverse) => {
    const previousPoint = previous || current;
    const nextPoint = next || current;
    const opposed = line(previousPoint, nextPoint);
    const angle = opposed.angle + (reverse ? Math.PI : 0);
    const length = opposed.length * 0.2;
    return { x: current.x + Math.cos(angle) * length, y: current.y + Math.sin(angle) * length };
  };
  return points.reduce((path, current, index, all) => {
    if (index === 0) return `M ${current.x},${current.y}`;
    const previous = all[index - 1];
    const cp1 = controlPoint(previous, all[index - 2], current, false);
    const cp2 = controlPoint(current, previous, all[index + 1], true);
    return `${path} C ${cp1.x},${cp1.y} ${cp2.x},${cp2.y} ${current.x},${current.y}`;
  }, '');
}

// Browser equivalent of Remotion's useVideoTexture for non-React Three.js
// compositions. The element and texture are cached by source so a frame
// callback can reuse GPU resources across the entire render.
export async function getVideoTexture(source, options = {}) {
  await ensureThree();
  if (typeof source !== 'string' || !source) throw new TypeError('getVideoTexture expects a source URL');
  const cached = videoTextureCache.get(source);
  if (cached) {
    seekVideoTexture(cached.video, options);
    return cached.texture;
  }
  const video = document.createElement('video');
  video.preload = 'auto';
  video.muted = options.muted ?? true;
  video.loop = options.loop ?? false;
  video.playsInline = true;
  video.src = source;
  const ready = video.readyState >= 2 ? Promise.resolve() : new Promise((resolve, reject) => {
    video.addEventListener('loadeddata', resolve, { once: true });
    video.addEventListener('error', () => reject(new Error(`failed to load video texture: ${source}`)), { once: true });
  });
  await ready;
  const texture = new THREE.VideoTexture(video);
  texture.colorSpace = THREE.SRGBColorSpace;
  videoTextureCache.set(source, { video, texture });
  seekVideoTexture(video, options);
  return texture;
}

// Naming-compatible entry point for adapters ported from @remotion/three.
export const useVideoTexture = getVideoTexture;

// Browser-compatible counterpart of @remotion/three's
// useOffthreadVideoTexture. The worker owns deterministic frame seeking, so
// this reuses the cached texture while injecting the current composition frame.
export async function getOffthreadVideoTexture(source, options = {}) {
  return getVideoTexture(source, {
    ...options,
    frame: options.frame ?? frame,
    fps: options.fps ?? videoConfig.fps,
  });
}

export const useOffthreadVideoTexture = getOffthreadVideoTexture;

function seekVideoTexture(video, options) {
  const frame = Number(options.frame);
  const fps = Number(options.fps ?? 30);
  if (Number.isFinite(frame) && Number.isFinite(fps) && fps > 0) {
    const time = Math.max(0, frame / fps);
    if (Math.abs(video.currentTime - time) > 1e-4) video.currentTime = time;
  }
}

export function releaseVideoTexture(source) {
  const cached = videoTextureCache.get(source);
  if (!cached) return false;
  cached.texture.dispose();
  cached.video.pause();
  cached.video.removeAttribute('src');
  cached.video.load();
  videoTextureCache.delete(source);
  return true;
}

// Optional browser ecosystem adapter. The core worker stays independent from
// lottie-web while applications can reuse any Lottie-compatible renderer.
export function registerLottieAdapter(adapter) {
  if (!adapter || typeof adapter.render !== 'function') {
    throw new TypeError('registerLottieAdapter expects an object with render()');
  }
  lottieAdapter = adapter;
}

const defaultLottieAdapter = {
  async render(element, state) {
    const lottie = (await (lottieModulePromise ??= import('lottie-web'))).default;
    let instance = lottieInstances.get(element);
    if (!instance || instance.src !== state.src) {
      instance?.animation.destroy();
      const preloaded = preloadedAssets.get(state.src);
      const animationData = preloaded && isJsonSource(state.src)
        ? await preloaded
        : undefined;
      const animation = lottie.loadAnimation({
        container: element,
        renderer: 'svg',
        loop: false,
        autoplay: false,
        ...(animationData ? { animationData } : { path: state.src }),
      });
      instance = { animation, src: state.src, ready: new Promise((resolve) => {
        animation.addEventListener('DOMLoaded', resolve, { once: true });
      }) };
      lottieInstances.set(element, instance);
    }
    await instance.ready;
    const totalFrames = Math.max(1, instance.animation.totalFrames || 1);
    const rawFrame = state.time * state.fps * state.playbackRate;
    const frame = state.loopBehavior === 'Loop'
      ? ((rawFrame % totalFrames) + totalFrames) % totalFrames
      : Math.max(0, Math.min(totalFrames - 1, rawFrame));
    element.style.visibility = state.loopBehavior === 'Unmount' && rawFrame >= totalFrames
      ? 'hidden' : '';
    instance.animation.goToAndStop(frame, true);
  },
};

lottieAdapter = defaultLottieAdapter;

export function listCompositions() {
  return [...new Set(['three_preview', ...compositions.keys()])];
}

// Remotion-compatible read-only hooks for browser compositions. They are
// updated at the start of every explicit render request, so adapters ported
// from React Three Fiber can use the familiar API without depending on React.
export function useCurrentFrame() {
  return frame;
}

export function useVideoConfig() {
  return { ...videoConfig };
}

// Small compatibility layer for Remotion's static-file helpers. The browser
// host owns URL resolution, so compositions stay portable between Vite,
// packaged Tauri assets, and a remote preview origin.
export function staticFile(path) {
  if (typeof path !== 'string' || !path.trim()) {
    throw new TypeError('staticFile expects a non-empty path');
  }
  return new URL(path.replace(/^\/+/, ''), document.baseURI).toString();
}

export function getStaticFiles() {
  return [...activeAssets];
}

// Studio-compatible watcher. Vite/Tauri integrations can dispatch the same
// event with `{files: [{name, lastModified}]}` when a static asset changes;
// headless export naturally remains a no-op because no watcher is attached.
export function watchStaticFile(fileName, callback) {
  if (typeof fileName !== 'string' || typeof callback !== 'function') {
    throw new TypeError('watchStaticFile expects a file name and callback');
  }
  const normalized = fileName.replace(/^\/+/, '');
  let previous;
  const listener = (event) => {
    const files = event.detail?.files;
    if (!Array.isArray(files)) return;
    const next = files.find((file) => file?.name === normalized);
    if (!next && previous) callback(null);
    if (next && (!previous || next.lastModified !== previous.lastModified)) callback(next);
    previous = next;
  };
  window.addEventListener('remotion_staticFilesChanged', listener);
  return { cancel: () => window.removeEventListener('remotion_staticFilesChanged', listener) };
}

// Remotion-compatible access to the current composition input props.
export function getInputProps() {
  return typeof structuredClone === 'function'
    ? structuredClone(activeProps)
    : JSON.parse(JSON.stringify(activeProps));
}

export function getRemotionEnvironment() {
  const isRendering = window.__DIOXUSCUT_HEADLESS_RENDER__ === true;
  return { isRendering, isStudio: !isRendering, isPlayer: false };
}

export const useRemotionEnvironment = getRemotionEnvironment;

// Explicit frame input keeps this scene deterministic for future exports.
async function renderDefaultFrame({ composition, frame: nextFrame, fps, props, width, height }) {
  await ensureThree();
  frame = nextFrame;
  if (Number.isFinite(width) && Number.isFinite(height)) {
    renderer.setSize(width, height, false);
    camera.aspect = width / Math.max(height, 1);
    camera.updateProjectionMatrix();
  }
  cube.rotation.x = nextFrame / Math.max(fps, 1) * 0.36;
  cube.rotation.y = nextFrame / Math.max(fps, 1) * 0.54;
  if (typeof props.color === 'string') cube.material.color.set(props.color);
  renderer.render(scene, camera);
  document.querySelector('#frame').textContent = `frame ${nextFrame}`;
  document.querySelector('#protocol').textContent = `composition ${composition}`;
}

async function syncMediaElements({ frame: nextFrame, fps }) {
  const timelineTime = nextFrame / Math.max(fps, 1);
  const pendingSeeks = [];
  for (const media of document.querySelectorAll('video, audio')) {
    const start = Number(media.dataset.timelineStart ?? 0);
    const duration = media.dataset.duration === undefined
      ? undefined
      : Number(media.dataset.duration);
    const end = duration === undefined ? undefined : start + duration;
    const active = timelineTime >= start && (end === undefined || timelineTime < end);
    media.style.visibility = active ? '' : 'hidden';
    if (!active) continue;

    const explicitTime = media.dataset.time ?? media.dataset.remotionSeek;
    const time = explicitTime === undefined
      ? undefined
      : Number(explicitTime);
    if (Number.isFinite(time) && Math.abs(media.currentTime - time) > 1e-4) {
      media.currentTime = Math.max(0, time);
      pendingSeeks.push(new Promise((resolve) => {
        const done = () => { media.removeEventListener('seeked', done); resolve(); };
        media.addEventListener('seeked', done, { once: true });
        setTimeout(done, 1000);
      }));
    }
    const volume = Number(media.dataset.volume ?? media.dataset.remotionVolume);
    if (Number.isFinite(volume)) media.volume = Math.max(0, Math.min(1, volume));
    const rate = Number(media.dataset.playbackRate ?? media.dataset.remotionPlaybackRate);
    if (Number.isFinite(rate) && rate > 0) media.playbackRate = rate;
    media.loop = media.hasAttribute('loop');
    media.pause();
  }
  await Promise.all(pendingSeeks);
}

function isJsonSource(source = '') {
  return source.split(/[?#]/, 1)[0].toLowerCase().endsWith('.json');
}

async function syncLottieElements({ frame: nextFrame, fps }) {
  if (!lottieAdapter) return;
  const elements = document.querySelectorAll('[data-dioxuscut-lottie]');
  await Promise.all([...elements].map(async (element) => {
    await lottieAdapter.render(element, {
      src: element.dataset.dioxuscutLottie,
      frame: nextFrame,
      fps,
      time: Number(element.dataset.time ?? 0),
      playbackRate: Number(element.dataset.playbackRate ?? 1),
      loopBehavior: element.dataset.loop ?? 'Loop',
    });
    element.dataset.frame = String(nextFrame);
  }));
}

async function syncCanvasImages(nextFrame) {
  const elements = document.querySelectorAll('[data-dioxuscut-canvas-image]');
  await Promise.all([...elements].map(async (element) => {
    const source = element.dataset.src;
    if (!source) return;
    const retries = Math.max(0, Number(element.dataset.maxRetries ?? 2));
    let asset;
    let lastError;
    for (let attempt = 0; attempt <= retries; attempt += 1) {
      try {
        asset = await (preloadedAssets.get(source) ?? preloadAssets([source]).then(() => preloadedAssets.get(source)));
        lastError = undefined;
        break;
      } catch (error) {
        lastError = error;
        preloadedAssets.delete(source);
        if (attempt < retries) await new Promise((resolve) => setTimeout(resolve, 50 * 2 ** attempt));
      }
    }
    if (lastError) {
      if (element.dataset.pauseWhenLoading === 'true') element.style.visibility = 'hidden';
      throw lastError;
    }
    const drawable = await asset;
    const width = Number(element.getAttribute('width')) || element.clientWidth || drawable.videoWidth || drawable.naturalWidth || 1;
    const height = Number(element.getAttribute('height')) || element.clientHeight || drawable.videoHeight || drawable.naturalHeight || 1;
    if (element.width !== width) element.width = width;
    if (element.height !== height) element.height = height;
    const context = element.getContext('2d');
    if (!context) return;
    context.clearRect(0, 0, width, height);
    const sourceWidth = drawable.videoWidth || drawable.naturalWidth || drawable.width || width;
    const sourceHeight = drawable.videoHeight || drawable.naturalHeight || drawable.height || height;
    const fit = element.dataset.fit ?? 'cover';
    const scale = fit === 'fill'
      ? { x: width / sourceWidth, y: height / sourceHeight }
      : fit === 'contain' || fit === 'scale-down'
        ? { x: Math.min(width / sourceWidth, height / sourceHeight), y: Math.min(width / sourceWidth, height / sourceHeight) }
        : fit === 'none'
          ? { x: 1, y: 1 }
          : { x: Math.max(width / sourceWidth, height / sourceHeight), y: Math.max(width / sourceWidth, height / sourceHeight) };
    const drawWidth = sourceWidth * scale.x;
    const drawHeight = sourceHeight * scale.y;
    context.drawImage(drawable, (width - drawWidth) / 2, (height - drawHeight) / 2, drawWidth, drawHeight);
    element.dataset.frame = String(nextFrame);
  }));
}

export async function renderFrame({ composition = 'three_preview', frame: nextFrame, fps = 30, props: inputProps = {}, assets = [], timeline = [], width, height, durationInFrames }) {
  const props = inputProps && typeof inputProps === 'object' ? inputProps : {};
  frame = nextFrame;
  activeAssets = Array.isArray(assets) ? [...assets] : [];
  activeProps = props;
  videoConfig = {
    ...videoConfig,
    fps,
    ...(Number.isFinite(width) ? { width } : {}),
    ...(Number.isFinite(height) ? { height } : {}),
    ...(Number.isFinite(durationInFrames) ? { durationInFrames } : {}),
  };
  // A cancelled gate belongs to the current frame only. Reset it before the
  // next request so a transient asset/render cancellation does not poison the
  // rest of the composition.
  renderGateError = null;
  await preloadAssets(assets);
  // Match Remotion's render-ready gate: a frame is not capturable until all
  // declared font faces have finished loading.
  await document.fonts.ready;
  if (timeline.length > 0) {
    for (const clip of timeline) {
      if (nextFrame < clip.start || nextFrame >= clip.start + clip.duration) continue;
      const render = compositions.get(clip.composition) ??
        (clip.composition === 'three_preview' ? renderDefaultFrame : undefined);
      if (!render) throw new Error(`unknown browser composition: ${clip.composition}`);
      await render({
        composition: clip.composition,
        frame: nextFrame - clip.start,
        fps,
        props: clip.props ?? {},
        assets,
        width,
        height,
        durationInFrames: clip.duration,
      });
    }
    await syncMediaElements({ frame: nextFrame, fps });
    await syncLottieElements({ frame: nextFrame, fps });
    await syncCanvasImages(nextFrame);
    await waitForRenderGates();
    return;
  }
  const customRender = compositions.get(composition);
  if (customRender) {
    const result = await customRender({
      composition,
      frame: nextFrame,
      fps,
      props,
      assets,
      width,
      height,
      durationInFrames,
    });
    await syncMediaElements({ frame: nextFrame, fps });
    await syncLottieElements({ frame: nextFrame, fps });
    await syncCanvasImages(nextFrame);
    await waitForRenderGates();
    return result;
  }
  const result = await renderDefaultFrame({ composition, frame: nextFrame, fps, props, width, height });
  await syncMediaElements({ frame: nextFrame, fps });
  await syncLottieElements({ frame: nextFrame, fps });
  await syncCanvasImages(nextFrame);
  await waitForRenderGates();
  return result;
}
window.dioxuscut = {
  renderFrame,
  registerComposition,
  registerThreeComposition,
  unregisterThreeComposition,
  listCompositions,
  delayRender,
  continueRender,
  cancelRender,
  useDelayRender,
  useBufferState,
  usePixelDensity,
  registerLottieAdapter,
  getVideoTexture,
  useVideoTexture,
  getOffthreadVideoTexture,
  useOffthreadVideoTexture,
  getImageDimensions,
  getVideoMetadata,
  parseMedia,
  parseWavMetadata,
  readMediaRange,
  getAudioDurationInSeconds,
  getAudioDuration,
  getAudioData,
  useAudioData,
  audioBufferToDataUrl,
  prefetch,
  usePreload,
  getWindowedAudioData,
  useWindowedAudioData,
  getWaveformPortion,
  visualizeAudioWaveform,
  visualizeAudio,
  createSmoothSvgPath,
  releaseVideoTexture,
  useCurrentFrame,
  useVideoConfig,
  staticFile,
  getStaticFiles,
  watchStaticFile,
  getInputProps,
  getRemotionEnvironment,
  useRemotionEnvironment,
};

function resize() {
  if (!renderer || !camera) return;
  const { width, height } = canvas.parentElement.getBoundingClientRect();
  renderer.setSize(width, height, false);
  camera.aspect = width / Math.max(height, 1);
  camera.updateProjectionMatrix();
}
window.addEventListener('resize', resize);
resize();

let frame = 0;
let videoConfig = { fps: 30, width: 1280, height: 720, durationInFrames: 150 };
let activeAssets = [];
let activeProps = {};
let playing = true;
let currentJobId = null;
let playbackStartedAt = performance.now();
let lastPlaybackFrame = -1;
let project = {
  version: 1,
  composition: 'three_preview',
  settings: { width: 1280, height: 720, fps: 30, duration: 150, backend: 'browser' },
  props: { color: '#6c63ff' }, assets: [], tracks: [],
};

function showMessage(message) {
  document.querySelector('#message').textContent = message;
}

function setFrame(nextFrame) {
  frame = Math.max(0, Math.min(nextFrame, project.settings.duration - 1));
  document.querySelector('#timeline-slider').max = project.settings.duration - 1;
  document.querySelector('#timeline-slider').value = frame;
  document.querySelector('#timeline-frame').textContent = `${frame} / ${project.settings.duration - 1}`;
  renderFrame({
    composition: project.composition,
    frame,
    fps: project.settings.fps,
    width: project.settings.width,
    height: project.settings.height,
    durationInFrames: project.settings.duration,
    props: project.props,
    assets: project.assets.map((asset) => asset.path),
    timeline: project.tracks.flatMap((track) => track.clips),
  }).catch((error) => showMessage(`preview error: ${error}`));
}

document.querySelector('#play-toggle').addEventListener('click', (event) => {
  playing = !playing;
  if (playing) playbackStartedAt = performance.now() - (frame * 1000 / project.settings.fps);
  event.currentTarget.textContent = playing ? 'Pause' : 'Play';
});
document.querySelector('#timeline-slider').addEventListener('input', (event) => {
  playing = false;
  lastPlaybackFrame = -1;
  document.querySelector('#play-toggle').textContent = 'Play';
  setFrame(Number(event.currentTarget.value));
});

async function refreshJob(id) {
  const job = await invoke('get_render_job', { id });
  if (!job) return;
  const progress = `${job.completed_frames}/${job.project.settings.duration}`;
  document.querySelector('#job').textContent = `${job.id} · ${job.status} · ${progress}`;
  const terminal = ['completed', 'failed', 'cancelled'].includes(job.status);
  document.querySelector('#cancel-render').disabled = terminal;
  if (terminal && job.error) showMessage(job.error);
}

async function refreshJobList() {
  const jobs = await invoke('list_render_jobs');
  const list = document.querySelector('#job-list');
  if (!jobs.length) { list.textContent = 'No jobs'; return; }
  list.replaceChildren(...jobs.map((job) => {
    const item = document.createElement('div');
    item.className = 'job-item';
    const progress = `${job.completed_frames}/${job.project.settings.duration}`;
    const label = document.createElement('span');
    label.textContent = `${job.id} · ${job.status} · ${progress}${job.error ? ` · ${job.error}` : ''}`;
    item.append(label);
    if (['failed', 'cancelled'].includes(job.status)) {
      const retry = document.createElement('button');
      retry.className = 'retry-job';
      retry.textContent = 'Retry';
      retry.addEventListener('click', async () => {
        try {
          currentJobId = await invoke('retry_render_job', { id: job.id });
          const output = await join(await tempDir(), `dioxuscut-${currentJobId}.mp4`);
          await invoke('start_render_job', { id: currentJobId, output });
          showMessage(`${currentJobId} rendering → ${output}`);
          await refreshJobList();
        } catch (error) { showMessage(`retry error: ${error}`); }
      });
      item.append(' ', retry);
    }
    return item;
  }));
}

setInterval(() => {
  if (currentJobId) refreshJob(currentJobId).catch((error) => showMessage(`job error: ${error}`));
  refreshJobList().catch((error) => showMessage(`queue error: ${error}`));
}, 250);

document.querySelector('#load-project').addEventListener('click', async () => {
  try {
    const selected = await open({ filters: [{ name: 'Dioxuscut project', extensions: ['json'] }] });
    if (!selected || Array.isArray(selected)) return;
    document.querySelector('#project-path').value = selected;
    project = await invoke('load_project', { path: selected });
    playbackStartedAt = performance.now();
    lastPlaybackFrame = -1;
    showMessage(`loaded ${project.composition}`);
    setFrame(frame);
  } catch (error) { showMessage(`load error: ${error}`); }
});

document.querySelector('#save-project').addEventListener('click', async () => {
  try {
    await ensureThree();
    project.props = {
      ...(project.props && typeof project.props === 'object' && !Array.isArray(project.props)
        ? project.props
        : {}),
      color: cube.material.color.getStyle(),
    };
    const selected = await save({
      defaultPath: document.querySelector('#project-path').value,
      filters: [{ name: 'Dioxuscut project', extensions: ['json'] }],
    });
    if (!selected) return;
    document.querySelector('#project-path').value = selected;
    await invoke('save_project', { path: selected, project });
    showMessage('project saved');
  } catch (error) { showMessage(`save error: ${error}`); }
});

document.querySelector('#submit-render').addEventListener('click', async () => {
  try {
    const id = await invoke('submit_project', { project });
    const output = await join(await tempDir(), `dioxuscut-${id}.mp4`);
    await invoke('start_render_job', { id, output });
    currentJobId = id;
    document.querySelector('#cancel-render').disabled = false;
    document.querySelector('#cancel-render').dataset.jobId = id;
    showMessage(`${id} rendering → ${output}`);
    await refreshJob(id);
    await refreshJobList();
  } catch (error) { showMessage(`queue error: ${error}`); }
});

document.querySelector('#cancel-render').addEventListener('click', async (event) => {
  const id = event.currentTarget.dataset.jobId;
  if (!id) return;
  try {
    await invoke('cancel_render_job', { id });
    await refreshJob(id);
    await refreshJobList();
    showMessage('render cancelled');
  } catch (error) { showMessage(`cancel error: ${error}`); }
});

function renderPreview() {
  if (window.__DIOXUSCUT_HEADLESS_RENDER__) return;
  if (playing) {
    const elapsed = Math.max(0, performance.now() - playbackStartedAt);
    const duration = Math.max(project.settings.duration, 1);
    const nextFrame = Math.floor(elapsed * project.settings.fps / 1000) % duration;
    if (nextFrame !== lastPlaybackFrame) {
      lastPlaybackFrame = nextFrame;
      setFrame(nextFrame);
    }
  }
  requestAnimationFrame(renderPreview);
}
renderPreview();

Promise.all([invoke('backend_capabilities'), invoke('web_worker_protocol')])
  .then(([capabilities, protocol]) => {
    document.querySelector('#backend').textContent = capabilities.browser_runtime
      ? 'Tauri · Three.js preview' : 'native preview';
    document.querySelector('#protocol').textContent = `worker protocol v${protocol.version}`;
  })
  .catch((error) => {
    document.querySelector('#backend').textContent = `bridge error: ${error}`;
  });

invoke('validate_frame_request', {
  request: { frame: 0, fps: 30, width: 1280, height: 720, props: {} },
}).catch((error) => { document.querySelector('#protocol').textContent = `protocol error: ${error}`; });
