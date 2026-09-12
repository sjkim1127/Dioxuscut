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
    "backend": "browser"
  },
  "props": { "color": "#6c63ff" },
  "assets": [],
  "tracks": []
}
```

The same `Project` model is shared by Dioxus, Tauri, and CLI integrations.

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

The CLI and Tauri host use the persistent worker protocol. Configure the
worker with:

```sh
export DIOXUSCUT_BROWSER_WORKER=$PWD/apps/studio-tauri/scripts/three-render-worker.mjs
export DIOXUSCUT_BROWSER_URL=http://localhost:1420
export DIOXUSCUT_BROWSER_CONCURRENCY=1
```

Use concurrency greater than one only after measuring the target composition;
Chromium startup and WebGL resource contention can make a larger pool slower.

For a headless readiness check, `dioxuscut serve` exposes `GET /health` as JSON.
