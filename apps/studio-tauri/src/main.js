import * as THREE from 'three';
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
const renderer = new THREE.WebGLRenderer({ canvas, antialias: true, alpha: false });
renderer.setPixelRatio(Math.min(window.devicePixelRatio, 2));
renderer.setClearColor(0x0b1020);
const scene = new THREE.Scene();
const camera = new THREE.PerspectiveCamera(45, 1, 0.1, 100);
camera.position.z = 3;
scene.add(new THREE.HemisphereLight(0x9bbcff, 0x182033, 2));
const cube = new THREE.Mesh(
  new THREE.BoxGeometry(1, 1, 1),
  new THREE.MeshStandardMaterial({ color: 0x6c63ff, roughness: 0.28, metalness: 0.35 }),
);
scene.add(cube);

// Browser compositions can replace the demo scene without changing the Rust
// protocol. Adapters may register Three.js, R3F, or another WebGL renderer.
const compositions = new Map();
const preloadedAssets = new Map();
const imageDimensionsCache = new Map();
const videoMetadataCache = new Map();
const audioDurationCache = new Map();
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

// Browser equivalent of Remotion's useVideoTexture for non-React Three.js
// compositions. The element and texture are cached by source so a frame
// callback can reuse GPU resources across the entire render.
export async function getVideoTexture(source, options = {}) {
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

// Explicit frame input keeps this scene deterministic for future exports.
function renderDefaultFrame({ composition, frame: nextFrame, fps, props, width, height }) {
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
  const result = renderDefaultFrame({ composition, frame: nextFrame, fps, props, width, height });
  await syncMediaElements({ frame: nextFrame, fps });
  await syncLottieElements({ frame: nextFrame, fps });
  await syncCanvasImages(nextFrame);
  await waitForRenderGates();
  return result;
}
window.dioxuscut = {
  renderFrame,
  registerComposition,
  listCompositions,
  delayRender,
  continueRender,
  cancelRender,
  registerLottieAdapter,
  getVideoTexture,
  useVideoTexture,
  getImageDimensions,
  getVideoMetadata,
  getAudioDurationInSeconds,
  getAudioDuration,
  releaseVideoTexture,
  useCurrentFrame,
  useVideoConfig,
};

function resize() {
  const { width, height } = canvas.parentElement.getBoundingClientRect();
  renderer.setSize(width, height, false);
  camera.aspect = width / Math.max(height, 1);
  camera.updateProjectionMatrix();
}
window.addEventListener('resize', resize);
resize();

let frame = 0;
let videoConfig = { fps: 30, width: 1280, height: 720, durationInFrames: 150 };
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
