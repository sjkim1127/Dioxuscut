# Changelog

## 0.2.0 - 2026-09-24

### Fixed

- Native and GPU project renders preserve timeline clips, and saved project asset paths remain portable.
- Browser timeline hooks are scoped to each clip, and overlapping clips composite at the requested frame size.
- Partial exports honor their requested frame ranges and report progress; audio volume keyframes follow composition time.
- Concurrent render jobs use isolated temporary directories.
- Remote asset downloads reject unsafe destinations, revalidate redirects, and enforce per-asset and per-project byte limits.

### Release

- Publish `dioxuscut-project` and `dioxuscut-charts` with the crates.io package graph. The release workflow checks its ordered package list against Cargo metadata.
- Build and publish Python wheels and the source distribution to PyPI from the same version tag.

### Known limitations

- Remotion 4.0.495 compatibility gaps remain tracked in issues #4 and #5.
- Rust dependency advisories involving the Tauri, GLib, and WebKit version graph remain tracked in issue #32.
