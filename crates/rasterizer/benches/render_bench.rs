//! Criterion benchmarks: CPU (`TinySkiaBackend`) vs GPU (`WgpuBackend`).
//!
//! Run all benchmarks:
//!   cargo bench -p dioxuscut-rasterizer --features gpu
//!
//! Run as unit-test smoke checks (no timing, just assert no panic):
//!   cargo bench -p dioxuscut-rasterizer --features gpu -- --test
//!
//! CPU-only (no GPU feature):
//!   cargo bench -p dioxuscut-rasterizer

use criterion::{criterion_group, criterion_main, BenchmarkId, Criterion};
use dioxuscut_rasterizer::{
    backend::{FrameConfig, RasterizerBackend},
    scene::{Color, GradientStop, Scene, SceneNode},
    tiny_skia_backend::TinySkiaBackend,
};

// ─────────────────────────────────────────────────────────────────
// Scene factories
// ─────────────────────────────────────────────────────────────────

/// Minimal "hello world" — one gradient background + 3 rects.
fn scene_hello_world() -> Scene {
    Scene {
        nodes: vec![
            SceneNode::LinearGradient {
                x: 0.0,
                y: 0.0,
                w: 1920.0,
                h: 1080.0,
                angle_deg: 135.0,
                stops: vec![
                    GradientStop {
                        position: 0.0,
                        color: Color::rgb(20, 20, 40),
                    },
                    GradientStop {
                        position: 1.0,
                        color: Color::rgb(80, 20, 120),
                    },
                ],
            },
            SceneNode::Rect {
                x: 100.0,
                y: 200.0,
                w: 600.0,
                h: 80.0,
                fill: Color::rgb(255, 200, 50),
                stroke: Some(Color::WHITE),
                stroke_width: 3.0,
                corner_radius: 12.0,
            },
            SceneNode::Circle {
                cx: 960.0,
                cy: 540.0,
                r: 200.0,
                fill: Color::rgba(100, 180, 255, 180),
                stroke: Some(Color::WHITE),
                stroke_width: 4.0,
            },
        ],
    }
}

/// Dense grid of 25 × 14 rects — simulates a cyberpunk/tile layout.
fn scene_grid(cols: u32, rows: u32) -> Scene {
    let mut nodes = Vec::with_capacity((cols * rows + 1) as usize);
    nodes.push(SceneNode::Rect {
        x: 0.0,
        y: 0.0,
        w: 1920.0,
        h: 1080.0,
        fill: Color::rgb(8, 8, 18),
        stroke: None,
        stroke_width: 0.0,
        corner_radius: 0.0,
    });
    let cell_w = 1920.0 / cols as f32;
    let cell_h = 1080.0 / rows as f32;
    for row in 0..rows {
        for col in 0..cols {
            let hue = (col * rows + row) as f32 / (cols * rows) as f32;
            let r = (255.0 * (1.0 - hue)) as u8;
            let g = (200.0 * hue) as u8;
            let b = 180u8;
            nodes.push(SceneNode::Rect {
                x: col as f32 * cell_w + 2.0,
                y: row as f32 * cell_h + 2.0,
                w: cell_w - 4.0,
                h: cell_h - 4.0,
                fill: Color::rgba(r, g, b, 210),
                stroke: Some(Color::rgba(255, 255, 255, 40)),
                stroke_width: 1.0,
                corner_radius: 4.0,
            });
        }
    }
    Scene { nodes }
}

/// Stacked radial and linear gradients.
fn scene_complex_gradients() -> Scene {
    let mut nodes = Vec::new();
    // Background linear gradient
    nodes.push(SceneNode::LinearGradient {
        x: 0.0,
        y: 0.0,
        w: 1920.0,
        h: 1080.0,
        angle_deg: 45.0,
        stops: vec![
            GradientStop {
                position: 0.0,
                color: Color::rgb(5, 5, 20),
            },
            GradientStop {
                position: 0.5,
                color: Color::rgb(50, 10, 80),
            },
            GradientStop {
                position: 1.0,
                color: Color::rgb(10, 60, 100),
            },
        ],
    });
    // Multiple radial gradients
    for i in 0..8u32 {
        let cx = 240.0 * (i % 4 + 1) as f32;
        let cy = 360.0 * (i / 4 + 1) as f32;
        nodes.push(SceneNode::RadialGradient {
            cx,
            cy,
            r: 180.0,
            stops: vec![
                GradientStop {
                    position: 0.0,
                    color: Color::rgba(255, 200, 50, 200),
                },
                GradientStop {
                    position: 1.0,
                    color: Color::rgba(255, 50, 100, 0),
                },
            ],
        });
    }
    // Foreground circles
    for i in 0..6u32 {
        nodes.push(SceneNode::Circle {
            cx: 300.0 + i as f32 * 280.0,
            cy: 540.0,
            r: 100.0,
            fill: Color::rgba(80, 200, 255, 120),
            stroke: Some(Color::rgba(255, 255, 255, 80)),
            stroke_width: 2.0,
        });
    }
    Scene { nodes }
}

/// A representative browser/native media scene with a single image texture.
fn scene_image_fallback() -> Scene {
    Scene {
        nodes: vec![
            SceneNode::Rect {
                x: 0.0,
                y: 0.0,
                w: 1920.0,
                h: 1080.0,
                fill: Color::rgb(12, 12, 20),
                stroke: None,
                stroke_width: 0.0,
                corner_radius: 0.0,
            },
            SceneNode::Image {
                src: "data:image/png;base64,iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAQAAAC1HAwCAAAAC0lEQVR42mNk+A8AAQUBAScY42YAAAAASUVORK5CYII=".into(),
                x: 160.0,
                y: 120.0,
                w: 1600.0,
                h: 840.0,
                fit: dioxuscut_rasterizer::scene::ImageFit::Cover,
                opacity: 0.9,
            },
        ],
    }
}

fn scene_image_source(src: String) -> Scene {
    let mut scene = scene_image_fallback();
    if let Some(SceneNode::Image { src: image_src, .. }) = scene.nodes.get_mut(1) {
        *image_src = src;
    }
    scene
}

// ─────────────────────────────────────────────────────────────────
// Bench groups
// ─────────────────────────────────────────────────────────────────

fn bench_cpu_scenes(c: &mut Criterion) {
    let backend = TinySkiaBackend::headless();

    let mut group = c.benchmark_group("cpu_scenes_1080p");
    group.sample_size(20);

    let scenes: &[(&str, Scene)] = &[
        ("hello_world", scene_hello_world()),
        ("grid_25x14", scene_grid(25, 14)),
        ("complex_gradients", scene_complex_gradients()),
        ("image_cpu_fallback", scene_image_fallback()),
    ];

    for (name, scene) in scenes {
        let config = FrameConfig::new(1920, 1080, 0, 30.0);
        group.bench_with_input(BenchmarkId::from_parameter(name), name, |b, _| {
            b.iter(|| backend.render_frame(scene, &config).unwrap())
        });
    }
    group.finish();
}

fn bench_cpu_resolutions(c: &mut Criterion) {
    let backend = TinySkiaBackend::headless();
    let scene = scene_hello_world();

    let mut group = c.benchmark_group("cpu_hello_world_resolutions");
    group.sample_size(15);

    let resolutions: &[(&str, u32, u32)] = &[
        ("480p", 854, 480),
        ("720p", 1280, 720),
        ("1080p", 1920, 1080),
        ("4k", 3840, 2160),
    ];

    for (label, w, h) in resolutions {
        let config = FrameConfig::new(*w, *h, 0, 30.0);
        group.bench_with_input(BenchmarkId::from_parameter(label), label, |b, _| {
            b.iter(|| backend.render_frame(&scene, &config).unwrap())
        });
    }
    group.finish();
}

#[cfg(feature = "gpu")]
fn bench_gpu_scenes(c: &mut Criterion) {
    use dioxuscut_rasterizer::wgpu_backend::WgpuBackend;

    let backend = match WgpuBackend::new() {
        Ok(b) => b,
        Err(e) => {
            eprintln!("GPU backend unavailable, skipping GPU bench: {e}");
            return;
        }
    };

    let mut group = c.benchmark_group("gpu_scenes_1080p");
    group.sample_size(20);

    let scenes: &[(&str, Scene)] = &[
        ("hello_world", scene_hello_world()),
        ("grid_25x14", scene_grid(25, 14)),
        ("complex_gradients", scene_complex_gradients()),
        ("image_texture", scene_image_fallback()),
    ];

    for (name, scene) in scenes {
        let config = FrameConfig::new(1920, 1080, 0, 30.0);
        group.bench_with_input(BenchmarkId::from_parameter(name), name, |b, _| {
            b.iter(|| backend.render_frame(scene, &config).unwrap())
        });
    }
    group.finish();
}

#[cfg(feature = "gpu")]
fn bench_gpu_image_cache(c: &mut Criterion) {
    use dioxuscut_rasterizer::wgpu_backend::WgpuBackend;

    let Ok(warm_backend) = WgpuBackend::new() else {
        eprintln!("GPU backend unavailable, skipping image cache benchmark");
        return;
    };
    let Ok(cold_backend) = WgpuBackend::new() else {
        return;
    };
    let config = FrameConfig::new(1920, 1080, 0, 30.0);
    let base = "data:image/png;base64,iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAQAAAC1HAwCAAAAC0lEQVR42mNk+A8AAQUBAScY42YAAAAASUVORK5CYII=";
    let warm_scene = scene_image_source(base.into());
    let cold_scenes: Vec<Scene> = (0..20)
        .map(|index| {
            scene_image_source(format!(
                "data:image/png;cache_key={index};base64,iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAQAAAC1HAwCAAAAC0lEQVR42mNk+A8AAQUBAScY42YAAAAASUVORK5CYII="
            ))
        })
        .collect();

    let mut group = c.benchmark_group("gpu_image_cache_1080p");
    group.sample_size(10);
    group.bench_function("warm_cache_hit", |b| {
        b.iter(|| warm_backend.render_frame(&warm_scene, &config).unwrap())
    });
    let mut index = 0usize;
    group.bench_function("cold_unique_upload", |b| {
        b.iter(|| {
            let scene = &cold_scenes[index % cold_scenes.len()];
            index += 1;
            cold_backend.render_frame(scene, &config).unwrap()
        })
    });
    group.finish();
}

#[cfg(feature = "gpu")]
fn bench_gpu_resolutions(c: &mut Criterion) {
    use dioxuscut_rasterizer::wgpu_backend::WgpuBackend;

    let backend = match WgpuBackend::new() {
        Ok(b) => b,
        Err(e) => {
            eprintln!("GPU backend unavailable, skipping GPU resolution bench: {e}");
            return;
        }
    };
    let scene = scene_hello_world();

    let mut group = c.benchmark_group("gpu_hello_world_resolutions");
    group.sample_size(15);

    let resolutions: &[(&str, u32, u32)] = &[
        ("480p", 854, 480),
        ("720p", 1280, 720),
        ("1080p", 1920, 1080),
        ("4k", 3840, 2160),
    ];

    for (label, w, h) in resolutions {
        let config = FrameConfig::new(*w, *h, 0, 30.0);
        group.bench_with_input(BenchmarkId::from_parameter(label), label, |b, _| {
            b.iter(|| backend.render_frame(&scene, &config).unwrap())
        });
    }
    group.finish();
}

#[cfg(feature = "gpu")]
fn bench_gpu_streaming(c: &mut Criterion) {
    use dioxuscut_rasterizer::wgpu_backend::WgpuBackend;

    let backend = match WgpuBackend::new() {
        Ok(b) => b,
        Err(e) => {
            eprintln!("GPU backend unavailable, skipping GPU streaming bench: {e}");
            return;
        }
    };

    let mut group = c.benchmark_group("gpu_streaming_throughput_1080p");
    group.sample_size(15);

    let frame_count = 10u32;
    let scene = scene_grid(25, 14);

    group.bench_function("sequential_render_frame_10_frames", |b| {
        b.iter(|| {
            for f in 0..frame_count {
                let config = FrameConfig::new(1920, 1080, f, 30.0);
                let _ = backend.render_frame(&scene, &config).unwrap();
            }
        })
    });

    group.bench_function("pipelined_render_stream_10_frames", |b| {
        b.iter(|| {
            backend
                .render_stream(
                    frame_count,
                    &|_f| Ok(scene.clone()),
                    &|f| FrameConfig::new(1920, 1080, f, 30.0),
                    &mut |_f, _rgba| Ok(()),
                )
                .unwrap();
        })
    });

    group.finish();
}

#[cfg(feature = "gpu")]
fn bench_gpu_concurrent_resolutions(c: &mut Criterion) {
    use dioxuscut_rasterizer::wgpu_backend::WgpuBackend;
    use std::sync::Arc;

    let backend = match WgpuBackend::new() {
        Ok(b) => Arc::new(b),
        Err(e) => {
            eprintln!("GPU backend unavailable, skipping concurrent resolution bench: {e}");
            return;
        }
    };
    let scene = Arc::new(scene_hello_world());
    let mut group = c.benchmark_group("gpu_concurrent_resolutions");
    group.sample_size(15);
    group.bench_function("720p_plus_1080p", |b| {
        b.iter(|| {
            let left_backend = Arc::clone(&backend);
            let right_backend = Arc::clone(&backend);
            let left_scene = Arc::clone(&scene);
            let right_scene = Arc::clone(&scene);
            let left = std::thread::spawn(move || {
                left_backend
                    .render_frame(&left_scene, &FrameConfig::new(1280, 720, 0, 30.0))
                    .unwrap()
            });
            let right = std::thread::spawn(move || {
                right_backend
                    .render_frame(&right_scene, &FrameConfig::new(1920, 1080, 0, 30.0))
                    .unwrap()
            });
            left.join().unwrap();
            right.join().unwrap();
        })
    });
    group.finish();
}

// ─────────────────────────────────────────────────────────────────
// Entry points
// ─────────────────────────────────────────────────────────────────

#[cfg(not(feature = "gpu"))]
criterion_group!(benches, bench_cpu_scenes, bench_cpu_resolutions);

#[cfg(feature = "gpu")]
criterion_group!(
    benches,
    bench_cpu_scenes,
    bench_cpu_resolutions,
    bench_gpu_scenes,
    bench_gpu_image_cache,
    bench_gpu_resolutions,
    bench_gpu_streaming,
    bench_gpu_concurrent_resolutions
);

criterion_main!(benches);
