import { invoke } from '@tauri-apps/api/core';

const app = document.querySelector('#app');
app.innerHTML = `<main><h1>Dioxuscut Studio</h1><p>Native and Three.js preview host</p><pre id="status">Loading backend contract…</pre></main>`;

const status = document.querySelector('#status');
Promise.all([invoke('backend_capabilities'), invoke('web_worker_protocol')])
  .then(([capabilities, protocol]) => {
    status.textContent = JSON.stringify({ capabilities, protocol }, null, 2);
  })
  .catch((error) => { status.textContent = `Studio bridge error: ${error}`; });
