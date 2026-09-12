# AI-first rendering contract

Dioxuscut treats the project file and render-job API as the primary interface.
The desktop preview is optional; an agent can run the same workflow through the
CLI or Tauri commands.

## Project input

Projects use the versioned `.dioxuscut.json` schema. A language-neutral JSON
Schema is available at `schemas/dioxuscut-project-v1.schema.json`.

```json
{
  "version": 1,
  "composition": "three_preview",
  "settings": {
    "width": 1280,
    "height": 720,
    "fps": 30,
    "duration": 150,
    "concurrency": 4,
    "frame_step": 1,
    "backend": "browser"
  },
  "props": { "color": "#6c63ff" },
  "assets": [
    { "id": "logo", "path": "assets/logo.png", "kind": "image" },
    { "id": "font", "path": "assets/Inter-Regular.woff2", "kind": "font" }
  ],
  "tracks": []
}
```

The same `Project` model is shared by Dioxus, Tauri, and CLI integrations.
Set `settings.concurrency` to a positive worker count when a project needs
reproducible parallelism; omit it for host-selected defaults.

Discover Native composition IDs without starting a UI:

```sh
DIOXUSCUT_JSON=1 dioxuscut list-compositions
```

Validate a project before submitting it:

```sh
dioxuscut validate-project project.dioxuscut.json
```

This performs the same strict Rust validation used by Tauri, including schema
version, dimensions, timing, backend, and unknown-field checks.

Render the validated project directly:

```sh
DIOXUSCUT_JSON=1 dioxuscut render-project project.dioxuscut.json --output out.mp4
```

`render-project` maps the project backend to the shared renderer and emits the
same machine-readable success/error result as `render`.

Video renders may use `--frame-step N` to follow Remotion's `everyNthFrame`
behavior: source frames are sampled every N frames and the output FPS is
reduced accordingly. Still-image codecs remain single-frame operations; zero
is always rejected.

For Browser projects, asset paths and timeline clips are passed to the worker
before the first frame. Images, fonts, and video metadata are loaded once and
reused across frames; a failed load is retried according to the Browser frame
retry policy. Active clips receive their local frame and clip props.

Assets with `kind: "audio"` are also forwarded as encoder inputs by
`render-project` and the Tauri job host, so an AI-generated project can declare
audio in the shared project file without a host-specific CLI flag.

## Job lifecycle

1. Submit the project with `submit_project` and retain the returned job id.
2. Start a browser job with `start_render_job(id, output)`.
3. Poll `get_render_job(id)` or `list_render_jobs()`.
4. Cancel active work with `cancel_render_job(id)`; cancellation reaches the
   FFmpeg/Chromium render control.
5. Retry a failed or cancelled job with `retry_render_job(id)`.

Valid progress states are `queued`, `preparing`, `rendering`, `encoding`,
`completed`, `failed`, and `cancelled`. A completed job includes its output path.

## Browser backend

The CLI and Tauri host use a persistent worker pool. Configure the pool with:

```sh
export DIOXUSCUT_BROWSER_WORKER=$PWD/apps/studio-tauri/scripts/three-render-worker.mjs
export DIOXUSCUT_BROWSER_URL=http://localhost:1420
export DIOXUSCUT_BROWSER_CONCURRENCY=4
# Optional: fail readiness or an async renderFrame() that exceeds this limit.
export DIOXUSCUT_BROWSER_FRAME_TIMEOUT_MS=30000
# Optional: retry a failed frame (default: 1 retry).
export DIOXUSCUT_BROWSER_FRAME_RETRIES=1
# Optional: lower-latency lossy browser capture for opaque video workloads.
export DIOXUSCUT_BROWSER_IMAGE_FORMAT=jpeg
export DIOXUSCUT_BROWSER_JPEG_QUALITY=90
```

The browser host exposes `window.dioxuscut.registerComposition(id, render)` and
`window.dioxuscut.listCompositions()`;
Three.js or React Three Fiber adapters can register composition-specific frame
functions without changing the Rust protocol. The function receives
`{frame, fps, props}` and may return a Promise for asynchronous asset loading.

Each worker owns one Chromium page and serializes its own requests; the shared
streaming pipeline schedules different frames across the pool while preserving
output order. Use concurrency greater than one only after measuring the target
composition; Chromium startup and WebGL resource contention can make a larger
pool slower.

PNG remains the default and is recommended for transparent or pixel-sensitive
stills. JPEG capture reduces browser-to-Rust payload size for opaque video
frames; FFmpeg still performs the final codec encoding.

For a headless readiness check, `dioxuscut serve` exposes `GET /health` as JSON.
Tauri hosts additionally expose `list_browser_compositions`, which performs a
worker handshake and returns the registered Browser composition IDs without
requiring the Studio UI.

For render automation, set `DIOXUSCUT_JSON=1`. Successful and failed `render`
commands then emit one machine-readable result line on stdout. Human tracing
remains on stderr, so an agent can parse stdout without log filtering.
