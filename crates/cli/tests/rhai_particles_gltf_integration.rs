#![cfg(feature = "rhai")]

use base64::Engine;
use dioxuscut_cli::rhai_runtime::RhaiComposition;
use dioxuscut_composition::{Composition, NativeCompositionContext};
use dioxuscut_rasterizer::SceneNode;
use serde_json::json;

#[test]
fn test_rhai_confetti_particles() {
    let script = r##"
        fn render(context, props) {
            let scene = scene();
            scene.confetti(#{
                count: 40,
                frame: context.frame,
                fps: context.fps,
                velocity: 500.0,
                colors: ["#ff0055", "#00ffcc"],
                shapes: ["rect", "circle", "star"]
            });
            scene.confetti_cannon("bottom_left", #{
                count: 20,
                frame: context.frame,
                fps: context.fps
            });
            return scene;
        }
    "##;

    let comp = RhaiComposition::from_source("test_confetti", script).expect("compile rhai");
    let ctx = NativeCompositionContext {
        width: 1920,
        height: 1080,
        fps: 30.0,
        duration_in_frames: 60,
    };
    let prepared = comp.prepare(&json!({}), ctx).expect("prepare");

    // Frame 0
    let scene_f0 = prepared.render(0).expect("render f0");
    assert_eq!(
        scene_f0.nodes.len(),
        60,
        "40 + 20 confetti particles emitted"
    );

    // Frame 15
    let scene_f15 = prepared.render(15).expect("render f15");
    assert_eq!(scene_f15.nodes.len(), 60);

    // Check that particles are wrapped in Group nodes with transforms
    for node in &scene_f15.nodes {
        match node {
            SceneNode::Group {
                transform,
                children,
                ..
            } => {
                assert!(transform.tx.is_finite());
                assert!(transform.ty.is_finite());
                assert_eq!(children.len(), 1);
            }
            other => panic!("Expected Group node for confetti particle, got: {other:?}"),
        }
    }
}

#[test]
fn test_rhai_mesh_3d_procedural() {
    let script = r##"
        fn render(context, props) {
            let scene = scene();
            // Render a rotating 3D Cube
            scene.mesh_3d("cube", #{
                x: 400.0,
                y: 300.0,
                size: 150.0,
                rotate_x: 25.0,
                rotate_y: 45.0,
                color: "#f59e0b"
            });

            // Render a 3D Sphere
            scene.mesh_3d("sphere", #{
                x: 800.0,
                y: 300.0,
                size: 100.0,
                rings: 8,
                sectors: 12,
                color: "#3b82f6"
            });

            // Render a 3D Torus
            scene.mesh_3d("torus", #{
                x: 1200.0,
                y: 300.0,
                size: 120.0,
                color: "#10b981"
            });
            return scene;
        }
    "##;

    let comp = RhaiComposition::from_source("test_mesh3d", script).expect("compile rhai");
    let ctx = NativeCompositionContext {
        width: 1920,
        height: 1080,
        fps: 30.0,
        duration_in_frames: 30,
    };
    let prepared = comp.prepare(&json!({}), ctx).expect("prepare");

    let scene = prepared.render(0).expect("render frame with 3d meshes");
    assert!(
        !scene.nodes.is_empty(),
        "3D meshes must produce scene nodes"
    );

    // All emitted nodes from mesh_3d should be Path nodes (polygons with Lambertian shading)
    let mut path_count = 0;
    for node in &scene.nodes {
        if let SceneNode::Path { d, fill, .. } = node {
            assert!(d.starts_with('M'), "Path must start with M");
            assert!(d.ends_with('Z'), "Path must close with Z");
            assert!(
                fill.is_some(),
                "Path must have Lambertian diffuse fill color"
            );
            path_count += 1;
        }
    }

    assert!(
        path_count >= 10,
        "Expected at least 10 projected faces across cube, sphere, and torus, got {path_count}"
    );
}

#[test]
fn test_rhai_gltf_loading_and_rendering() {
    // Generate minimal valid embedded glTF JSON with a 3D triangle
    let mut bin = Vec::new();
    for &(x, y, z) in &[
        (-1.0f32, 0.0f32, 0.0f32),
        (1.0f32, 0.0f32, 0.0f32),
        (0.0f32, 1.0f32, 0.0f32),
    ] {
        bin.extend_from_slice(&x.to_le_bytes());
        bin.extend_from_slice(&y.to_le_bytes());
        bin.extend_from_slice(&z.to_le_bytes());
    }
    let b64 = base64::engine::general_purpose::STANDARD.encode(&bin);

    let gltf_json = format!(
        r#"{{
            "asset": {{ "version": "2.0" }},
            "buffers": [{{ "byteLength": {}, "uri": "data:application/octet-stream;base64,{}" }}],
            "bufferViews": [{{ "buffer": 0, "byteOffset": 0, "byteLength": {} }}],
            "accessors": [{{ "bufferView": 0, "byteOffset": 0, "componentType": 5126, "count": 3, "type": "VEC3" }}],
            "meshes": [{{
                "name": "Triangle3D",
                "primitives": [{{ "attributes": {{ "POSITION": 0 }} }}]
            }}]
        }}"#,
        bin.len(),
        b64,
        bin.len()
    );

    let script = format!(
        r##"
        fn render(context, props) {{
            let scene = scene();
            let gltf_str = `{}`;
            scene.gltf(gltf_str, #{{
                x: 960.0,
                y: 540.0,
                scale: 150.0,
                rotate_y: 30.0,
                color: "#ec4899"
            }});
            return scene;
        }}
        "##,
        gltf_json
    );

    let comp = RhaiComposition::from_source("test_gltf", &script).expect("compile rhai");
    let ctx = NativeCompositionContext {
        width: 1920,
        height: 1080,
        fps: 30.0,
        duration_in_frames: 30,
    };
    let prepared = comp.prepare(&json!({}), ctx).expect("prepare");

    let scene = prepared.render(0).expect("render gltf frame");

    assert_eq!(scene.nodes.len(), 1, "Expected 1 projected triangle face");
    if let SceneNode::Path { d, fill, .. } = &scene.nodes[0] {
        assert!(d.starts_with('M'));
        assert!(fill.is_some());
    } else {
        panic!("Expected Path node for projected 3D face");
    }
}
