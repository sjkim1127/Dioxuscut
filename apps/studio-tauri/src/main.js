import * as THREE from 'three';
import { invoke } from '@tauri-apps/api/core';
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
      <span id="message"></span>
    </section>
    <section class="preview"><canvas id="preview-canvas"></canvas></section>
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

// Explicit frame input keeps this scene deterministic for future exports.
export function renderFrame({ frame: nextFrame, fps = 30, props = {} }) {
  frame = nextFrame;
  cube.rotation.x = nextFrame / Math.max(fps, 1) * 0.36;
  cube.rotation.y = nextFrame / Math.max(fps, 1) * 0.54;
  if (typeof props.color === 'string') cube.material.color.set(props.color);
  renderer.render(scene, camera);
  document.querySelector('#frame').textContent = `frame ${nextFrame}`;
}
window.dioxuscut = { renderFrame };

function resize() {
  const { width, height } = canvas.parentElement.getBoundingClientRect();
  renderer.setSize(width, height, false);
  camera.aspect = width / Math.max(height, 1);
  camera.updateProjectionMatrix();
}
window.addEventListener('resize', resize);
resize();

let frame = 0;
let project = {
  version: 1,
  composition: 'three_preview',
  settings: { width: 1280, height: 720, fps: 30, duration: 150, backend: 'browser' },
  props: { color: '#6c63ff' }, assets: [], tracks: [],
};

function showMessage(message) {
  document.querySelector('#message').textContent = message;
}

document.querySelector('#load-project').addEventListener('click', async () => {
  try {
    project = await invoke('load_project', { path: document.querySelector('#project-path').value });
    showMessage(`loaded ${project.composition}`);
    renderFrame({ frame, fps: project.settings.fps, props: project.props });
  } catch (error) { showMessage(`load error: ${error}`); }
});

document.querySelector('#save-project').addEventListener('click', async () => {
  try {
    project.props = { color: cube.material.color.getStyle() };
    await invoke('save_project', { path: document.querySelector('#project-path').value, project });
    showMessage('project saved');
  } catch (error) { showMessage(`save error: ${error}`); }
});

document.querySelector('#submit-render').addEventListener('click', async () => {
  try {
    const id = await invoke('submit_project', { project });
    showMessage(`${id} queued`);
    const job = await invoke('get_render_job', { id });
    if (job) document.querySelector('#job').textContent = `${job.id} · ${job.status}`;
  } catch (error) { showMessage(`queue error: ${error}`); }
});

function renderPreview() {
  renderFrame({ frame, fps: 30 });
  frame++;
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
