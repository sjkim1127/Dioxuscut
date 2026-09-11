//! Native glTF 2.0 and GLB 3D model loader.
//!
//! Parses 3D meshes, vertex buffers, triangle faces, and PBR materials
//! into [`Mesh3D`] for browserless software and GPU 3D rendering.

use crate::mesh3d::{Mesh3D, Vec3};
use crate::scene::Color;
use base64::Engine;
use serde::Deserialize;
use std::collections::HashMap;

/// Result of loading a glTF model.
#[derive(Debug, Clone)]
pub struct GltfModel {
    pub name: String,
    pub mesh: Mesh3D,
    pub base_color: Color,
}

#[derive(Deserialize, Debug)]
struct GltfRoot {
    #[serde(default)]
    buffers: Vec<GltfBuffer>,
    #[serde(default, rename = "bufferViews")]
    buffer_views: Vec<GltfBufferView>,
    #[serde(default)]
    accessors: Vec<GltfAccessor>,
    #[serde(default)]
    meshes: Vec<GltfMeshDef>,
    #[serde(default)]
    materials: Vec<GltfMaterial>,
}

#[derive(Deserialize, Debug)]
#[allow(dead_code)]
struct GltfBuffer {
    #[serde(default, rename = "byteLength")]
    byte_length: usize,
    #[serde(default)]
    uri: Option<String>,
}

#[derive(Deserialize, Debug)]
#[allow(dead_code)]
struct GltfBufferView {
    buffer: usize,
    #[serde(default, rename = "byteOffset")]
    byte_offset: usize,
    #[serde(rename = "byteLength")]
    byte_length: usize,
}

#[derive(Deserialize, Debug)]
struct GltfAccessor {
    #[serde(default, rename = "bufferView")]
    buffer_view: Option<usize>,
    #[serde(default, rename = "byteOffset")]
    byte_offset: usize,
    #[serde(rename = "componentType")]
    component_type: u32,
    count: usize,
    #[serde(rename = "type")]
    accessor_type: String,
}

#[derive(Deserialize, Debug)]
struct GltfMeshDef {
    #[serde(default)]
    name: Option<String>,
    #[serde(default)]
    primitives: Vec<GltfPrimitive>,
}

#[derive(Deserialize, Debug)]
struct GltfPrimitive {
    attributes: HashMap<String, usize>,
    #[serde(default)]
    indices: Option<usize>,
    #[serde(default)]
    material: Option<usize>,
}

#[derive(Deserialize, Debug)]
struct GltfMaterial {
    #[serde(default, rename = "pbrMetallicRoughness")]
    pbr: Option<GltfPbr>,
}

#[derive(Deserialize, Debug, Default)]
struct GltfPbr {
    #[serde(default, rename = "baseColorFactor")]
    base_color_factor: Option<[f32; 4]>,
}

/// Load a glTF model from a JSON string, optionally with external/binary buffer data.
pub fn parse_gltf(json_str: &str, bin_buffer: Option<&[u8]>) -> Result<GltfModel, String> {
    let root: GltfRoot =
        serde_json::from_str(json_str).map_err(|e| format!("glTF JSON parse error: {e}"))?;

    // Resolve buffers
    let mut resolved_buffers: Vec<Vec<u8>> = Vec::new();
    for (i, buf) in root.buffers.iter().enumerate() {
        if let Some(ref uri) = buf.uri {
            if let Some(base64_data) = uri.strip_prefix("data:application/octet-stream;base64,") {
                let bytes = base64::engine::general_purpose::STANDARD
                    .decode(base64_data)
                    .map_err(|e| format!("Base64 decode error in buffer {i}: {e}"))?;
                resolved_buffers.push(bytes);
            } else if let Some(base64_data) =
                uri.strip_prefix("data:application/gltf-buffer;base64,")
            {
                let bytes = base64::engine::general_purpose::STANDARD
                    .decode(base64_data)
                    .map_err(|e| format!("Base64 decode error in buffer {i}: {e}"))?;
                resolved_buffers.push(bytes);
            } else {
                return Err(format!(
                    "External file URIs not supported in sandboxed glTF parser: {uri}"
                ));
            }
        } else if let Some(bin) = bin_buffer {
            resolved_buffers.push(bin.to_vec());
        } else {
            return Err(format!(
                "Buffer {i} has no URI and no binary chunk was provided"
            ));
        }
    }

    // Extract first mesh
    let mesh_def = root
        .meshes
        .first()
        .ok_or_else(|| "glTF file contains no meshes".to_string())?;
    let prim = mesh_def
        .primitives
        .first()
        .ok_or_else(|| "Mesh has no primitives".to_string())?;

    // Extract POSITION attribute
    let pos_accessor_idx = prim
        .attributes
        .get("POSITION")
        .copied()
        .ok_or_else(|| "Primitive missing POSITION attribute".to_string())?;

    let vertices = read_vec3_accessor(&root, &resolved_buffers, pos_accessor_idx)?;

    // Extract indices (faces)
    let faces = if let Some(indices_idx) = prim.indices {
        read_indices_accessor(&root, &resolved_buffers, indices_idx)?
    } else {
        // Non-indexed: groups of 3 vertices form a triangle
        (0..vertices.len() / 3)
            .map(|i| vec![i * 3, i * 3 + 1, i * 3 + 2])
            .collect()
    };

    // Extract base color
    let mut base_color = Color::rgb(0x3b, 0x82, 0xf6);
    if let Some(mat_idx) = prim.material {
        if let Some(mat) = root.materials.get(mat_idx) {
            if let Some(ref pbr) = mat.pbr {
                if let Some(factor) = pbr.base_color_factor {
                    base_color = Color::rgba(
                        (factor[0].clamp(0.0, 1.0) * 255.0) as u8,
                        (factor[1].clamp(0.0, 1.0) * 255.0) as u8,
                        (factor[2].clamp(0.0, 1.0) * 255.0) as u8,
                        (factor[3].clamp(0.0, 1.0) * 255.0) as u8,
                    );
                }
            }
        }
    }

    let mut mesh = Mesh3D::new(vertices, faces);
    normalize_mesh(&mut mesh);

    Ok(GltfModel {
        name: mesh_def.name.clone().unwrap_or_else(|| "Model3D".into()),
        mesh,
        base_color,
    })
}

/// Load a binary `.glb` model from raw bytes.
pub fn parse_glb(bytes: &[u8]) -> Result<GltfModel, String> {
    if bytes.len() < 12 {
        return Err("Invalid GLB: file too short".into());
    }

    let magic = u32::from_le_bytes(bytes[0..4].try_into().unwrap());
    if magic != 0x4654_6C67 {
        // "glTF"
        return Err(format!("Invalid GLB magic header: {magic:#x}"));
    }

    let version = u32::from_le_bytes(bytes[4..8].try_into().unwrap());
    if version != 2 {
        return Err(format!("Unsupported GLB version: {version}"));
    }

    let mut offset = 12;
    let mut json_str = None;
    let mut bin_chunk = None;

    while offset + 8 <= bytes.len() {
        let chunk_len = u32::from_le_bytes(bytes[offset..offset + 4].try_into().unwrap()) as usize;
        let chunk_type = u32::from_le_bytes(bytes[offset + 4..offset + 8].try_into().unwrap());
        offset += 8;

        if offset + chunk_len > bytes.len() {
            return Err("GLB chunk extends past end of file".into());
        }

        let chunk_data = &bytes[offset..offset + chunk_len];

        match chunk_type {
            0x4E4F_534A => {
                // "JSON"
                let s = std::str::from_utf8(chunk_data)
                    .map_err(|e| format!("Invalid UTF-8 in GLB JSON chunk: {e}"))?;
                json_str = Some(s);
            }
            0x004E_4942 => {
                // "BIN\0"
                bin_chunk = Some(chunk_data);
            }
            _ => {}
        }

        offset += chunk_len;
    }

    let json = json_str.ok_or_else(|| "GLB contains no JSON chunk".to_string())?;
    parse_gltf(json, bin_chunk)
}

fn read_vec3_accessor(
    root: &GltfRoot,
    buffers: &[Vec<u8>],
    accessor_idx: usize,
) -> Result<Vec<Vec3>, String> {
    let accessor = root
        .accessors
        .get(accessor_idx)
        .ok_or_else(|| format!("Invalid accessor index {accessor_idx}"))?;

    if accessor.accessor_type != "VEC3" || accessor.component_type != 5126 {
        // 5126 = FLOAT
        return Err("POSITION accessor must be VEC3 of FLOAT".into());
    }

    let bv_idx = accessor
        .buffer_view
        .ok_or_else(|| "Accessor has no bufferView".to_string())?;
    let bv = root
        .buffer_views
        .get(bv_idx)
        .ok_or_else(|| format!("Invalid bufferView index {bv_idx}"))?;

    let buf = buffers
        .get(bv.buffer)
        .ok_or_else(|| format!("Invalid buffer index {}", bv.buffer))?;

    let start = bv.byte_offset + accessor.byte_offset;
    let float_bytes = 12; // 3 * 4 bytes
    let total_bytes = accessor.count * float_bytes;

    if start + total_bytes > buf.len() {
        return Err("Buffer overrun reading VEC3 positions".into());
    }

    let mut vertices = Vec::with_capacity(accessor.count);
    for i in 0..accessor.count {
        let o = start + i * float_bytes;
        let x = f32::from_le_bytes(buf[o..o + 4].try_into().unwrap());
        let y = f32::from_le_bytes(buf[o + 4..o + 8].try_into().unwrap());
        let z = f32::from_le_bytes(buf[o + 8..o + 12].try_into().unwrap());
        vertices.push(Vec3::new(x, y, z));
    }

    Ok(vertices)
}

fn read_indices_accessor(
    root: &GltfRoot,
    buffers: &[Vec<u8>],
    accessor_idx: usize,
) -> Result<Vec<Vec<usize>>, String> {
    let accessor = root
        .accessors
        .get(accessor_idx)
        .ok_or_else(|| format!("Invalid accessor index {accessor_idx}"))?;

    let bv_idx = accessor
        .buffer_view
        .ok_or_else(|| "Accessor has no bufferView".to_string())?;
    let bv = root
        .buffer_views
        .get(bv_idx)
        .ok_or_else(|| format!("Invalid bufferView index {bv_idx}"))?;

    let buf = buffers
        .get(bv.buffer)
        .ok_or_else(|| format!("Invalid buffer index {}", bv.buffer))?;

    let start = bv.byte_offset + accessor.byte_offset;
    let mut flat_indices = Vec::with_capacity(accessor.count);

    match accessor.component_type {
        5123 => {
            // UNSIGNED_SHORT (u16)
            let item_size = 2;
            if start + accessor.count * item_size > buf.len() {
                return Err("Buffer overrun reading u16 indices".into());
            }
            for i in 0..accessor.count {
                let o = start + i * item_size;
                let idx = u16::from_le_bytes(buf[o..o + 2].try_into().unwrap());
                flat_indices.push(idx as usize);
            }
        }
        5125 => {
            // UNSIGNED_INT (u32)
            let item_size = 4;
            if start + accessor.count * item_size > buf.len() {
                return Err("Buffer overrun reading u32 indices".into());
            }
            for i in 0..accessor.count {
                let o = start + i * item_size;
                let idx = u32::from_le_bytes(buf[o..o + 4].try_into().unwrap());
                flat_indices.push(idx as usize);
            }
        }
        5121 => {
            // UNSIGNED_BYTE (u8)
            for i in 0..accessor.count {
                flat_indices.push(buf[start + i] as usize);
            }
        }
        other => return Err(format!("Unsupported index componentType: {other}")),
    }

    let mut faces = Vec::with_capacity(flat_indices.len() / 3);
    for chunk in flat_indices.chunks(3) {
        if chunk.len() == 3 {
            faces.push(chunk.to_vec());
        }
    }

    Ok(faces)
}

/// Center the mesh at origin (0, 0, 0) and scale to unit radius.
fn normalize_mesh(mesh: &mut Mesh3D) {
    if mesh.vertices.is_empty() {
        return;
    }

    let (mut min_x, mut max_x) = (f32::INFINITY, f32::NEG_INFINITY);
    let (mut min_y, mut max_y) = (f32::INFINITY, f32::NEG_INFINITY);
    let (mut min_z, mut max_z) = (f32::INFINITY, f32::NEG_INFINITY);

    for v in &mesh.vertices {
        min_x = min_x.min(v.x);
        max_x = max_x.max(v.x);
        min_y = min_y.min(v.y);
        max_y = max_y.max(v.y);
        min_z = min_z.min(v.z);
        max_z = max_z.max(v.z);
    }

    let cx = (min_x + max_x) * 0.5;
    let cy = (min_y + max_y) * 0.5;
    let cz = (min_z + max_z) * 0.5;

    let span_x = (max_x - min_x).abs();
    let span_y = (max_y - min_y).abs();
    let span_z = (max_z - min_z).abs();
    let max_span = span_x.max(span_y).max(span_z).max(1e-4);

    let scale = 1.0 / max_span;

    for v in &mut mesh.vertices {
        v.x = (v.x - cx) * scale;
        v.y = (v.y - cy) * scale;
        v.z = (v.z - cz) * scale;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_embedded_gltf_triangle() {
        // Create a minimal triangle buffer
        // 3 vertices (3 * 3 * 4 = 36 bytes)
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

        let json = format!(
            r#"{{
                "asset": {{ "version": "2.0" }},
                "buffers": [{{ "byteLength": {}, "uri": "data:application/octet-stream;base64,{}" }}],
                "bufferViews": [{{ "buffer": 0, "byteOffset": 0, "byteLength": {} }}],
                "accessors": [{{ "bufferView": 0, "byteOffset": 0, "componentType": 5126, "count": 3, "type": "VEC3" }}],
                "meshes": [{{
                    "name": "TestTriangle",
                    "primitives": [{{ "attributes": {{ "POSITION": 0 }} }}]
                }}]
            }}"#,
            bin.len(),
            b64,
            bin.len()
        );

        let model = parse_gltf(&json, None).expect("parse gltf");
        assert_eq!(model.name, "TestTriangle");
        assert_eq!(model.mesh.vertices.len(), 3);
        assert_eq!(model.mesh.faces.len(), 1);
        assert_eq!(model.mesh.faces[0], vec![0, 1, 2]);
    }
}
