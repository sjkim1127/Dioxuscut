//! Native glTF 2.0 and GLB 3D model loader.
//!
//! Parses 3D meshes, vertex buffers, triangle faces, PBR materials,
//! and skeletal bone animations into [`Mesh3D`] for browserless 3D rendering.

use crate::mesh3d::{Mat4, Mesh3D, Quat, SkinnedVertex, Vec3};
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
    pub skinned_vertices: Vec<SkinnedVertex>,
    pub skin: Option<GltfSkinData>,
    pub nodes: Vec<GltfNodeData>,
    pub animations: Vec<GltfAnimationData>,
    pub rest_center: Vec3,
    pub rest_scale: f32,
}

#[derive(Debug, Clone)]
pub struct GltfSkinData {
    pub joints: Vec<usize>,
    pub inverse_bind_matrices: Vec<Mat4>,
}

#[derive(Debug, Clone)]
pub struct GltfNodeData {
    pub name: String,
    pub children: Vec<usize>,
    pub translation: Vec3,
    pub rotation: Quat,
    pub scale: Vec3,
}

#[derive(Debug, Clone)]
pub struct GltfAnimationData {
    pub name: String,
    pub duration: f32,
    pub channels: Vec<GltfChannelData>,
}

#[derive(Debug, Clone)]
pub struct GltfChannelData {
    pub target_node: usize,
    pub property: AnimationProperty,
    pub timestamps: Vec<f32>,
    pub values: Vec<[f32; 4]>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AnimationProperty {
    Translation,
    Rotation,
    Scale,
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
    #[serde(default)]
    nodes: Vec<GltfNodeDef>,
    #[serde(default)]
    skins: Vec<GltfSkinDef>,
    #[serde(default)]
    animations: Vec<GltfAnimationDef>,
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

#[derive(Deserialize, Debug)]
struct GltfNodeDef {
    #[serde(default)]
    name: Option<String>,
    #[serde(default)]
    children: Vec<usize>,
    #[serde(default)]
    translation: Option<[f32; 3]>,
    #[serde(default)]
    rotation: Option<[f32; 4]>,
    #[serde(default)]
    scale: Option<[f32; 3]>,
}

#[derive(Deserialize, Debug)]
struct GltfSkinDef {
    #[serde(default)]
    #[allow(dead_code)]
    name: Option<String>,
    #[serde(rename = "inverseBindMatrices")]
    inverse_bind_matrices: Option<usize>,
    #[serde(default)]
    joints: Vec<usize>,
}

#[derive(Deserialize, Debug)]
struct GltfAnimationDef {
    #[serde(default)]
    name: Option<String>,
    #[serde(default)]
    channels: Vec<GltfAnimChannelDef>,
    #[serde(default)]
    samplers: Vec<GltfAnimSamplerDef>,
}

#[derive(Deserialize, Debug)]
struct GltfAnimChannelDef {
    sampler: usize,
    target: GltfAnimTargetDef,
}

#[derive(Deserialize, Debug)]
struct GltfAnimTargetDef {
    node: Option<usize>,
    path: String,
}

#[derive(Deserialize, Debug)]
struct GltfAnimSamplerDef {
    input: usize,
    output: usize,
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

    // Parse JOINTS_0 & WEIGHTS_0
    let joints_opt = prim
        .attributes
        .get("JOINTS_0")
        .and_then(|&idx| read_vec4_u16_accessor(&root, &resolved_buffers, idx).ok());

    let weights_opt = prim
        .attributes
        .get("WEIGHTS_0")
        .and_then(|&idx| read_vec4_f32_accessor(&root, &resolved_buffers, idx).ok());

    let skinned_vertices = if let (Some(joints), Some(weights)) = (joints_opt, weights_opt) {
        vertices
            .iter()
            .enumerate()
            .map(|(i, &pos)| SkinnedVertex {
                position: pos,
                normal: Vec3::new(0.0, 1.0, 0.0),
                joints: joints.get(i).copied().unwrap_or([0, 0, 0, 0]),
                weights: weights.get(i).copied().unwrap_or([1.0, 0.0, 0.0, 0.0]),
            })
            .collect()
    } else {
        vertices
            .iter()
            .map(|&pos| SkinnedVertex {
                position: pos,
                normal: Vec3::new(0.0, 1.0, 0.0),
                joints: [0, 0, 0, 0],
                weights: [1.0, 0.0, 0.0, 0.0],
            })
            .collect()
    };

    // Parse Nodes
    let mut nodes = Vec::with_capacity(root.nodes.len());
    for (i, n) in root.nodes.iter().enumerate() {
        let name = n.name.clone().unwrap_or_else(|| format!("Node_{i}"));
        let translation = n
            .translation
            .map(|t| Vec3::new(t[0], t[1], t[2]))
            .unwrap_or_default();
        let rotation = n
            .rotation
            .map(|r| Quat::new(r[0], r[1], r[2], r[3]).normalize())
            .unwrap_or(Quat::IDENTITY);
        let scale = n
            .scale
            .map(|s| Vec3::new(s[0], s[1], s[2]))
            .unwrap_or(Vec3::new(1.0, 1.0, 1.0));

        nodes.push(GltfNodeData {
            name,
            children: n.children.clone(),
            translation,
            rotation,
            scale,
        });
    }

    // Parse Skin
    let skin = if let Some(skin_def) = root.skins.first() {
        let ibms = if let Some(ibm_idx) = skin_def.inverse_bind_matrices {
            read_mat4_accessor(&root, &resolved_buffers, ibm_idx).unwrap_or_default()
        } else {
            vec![Mat4::IDENTITY; skin_def.joints.len()]
        };
        Some(GltfSkinData {
            joints: skin_def.joints.clone(),
            inverse_bind_matrices: ibms,
        })
    } else {
        None
    };

    // Parse Animations
    let mut animations = Vec::new();
    for anim_def in &root.animations {
        let mut channels = Vec::new();
        let mut max_time = 0.0f32;

        for ch in &anim_def.channels {
            let Some(target_node) = ch.target.node else {
                continue;
            };
            let property = match ch.target.path.as_str() {
                "translation" => AnimationProperty::Translation,
                "rotation" => AnimationProperty::Rotation,
                "scale" => AnimationProperty::Scale,
                _ => continue,
            };

            let Some(sampler) = anim_def.samplers.get(ch.sampler) else {
                continue;
            };

            let Ok(timestamps) = read_f32_accessor(&root, &resolved_buffers, sampler.input) else {
                continue;
            };
            let Ok(values) =
                read_anim_values_accessor(&root, &resolved_buffers, sampler.output, property)
            else {
                continue;
            };

            if let Some(&last) = timestamps.last() {
                if last > max_time {
                    max_time = last;
                }
            }

            channels.push(GltfChannelData {
                target_node,
                property,
                timestamps,
                values,
            });
        }

        animations.push(GltfAnimationData {
            name: anim_def.name.clone().unwrap_or_else(|| "Animation".into()),
            duration: max_time,
            channels,
        });
    }

    let (rest_center, rest_scale) = compute_bounds(&vertices);

    let mut mesh = Mesh3D::new(vertices, faces);
    apply_normalization(&mut mesh, rest_center, rest_scale);

    Ok(GltfModel {
        name: mesh_def.name.clone().unwrap_or_else(|| "Model3D".into()),
        mesh,
        base_color,
        skinned_vertices,
        skin,
        nodes,
        animations,
        rest_center,
        rest_scale,
    })
}

impl GltfModel {
    /// Sample a skinned skeletal animation pose at `time_sec` and return the deformed [`Mesh3D`].
    pub fn sample_pose(&self, time_sec: f32) -> Mesh3D {
        let (Some(skin), Some(anim)) = (&self.skin, self.animations.first()) else {
            return self.mesh.clone();
        };

        if self.skinned_vertices.is_empty() || skin.joints.is_empty() || self.nodes.is_empty() {
            return self.mesh.clone();
        }

        let duration = anim.duration.max(0.001);
        let time = if duration > 0.0 {
            let t = time_sec % duration;
            if t == 0.0 && time_sec > 0.0 {
                duration
            } else {
                t
            }
        } else {
            0.0
        };

        let mut local_t: Vec<Vec3> = self.nodes.iter().map(|n| n.translation).collect();
        let mut local_r: Vec<Quat> = self.nodes.iter().map(|n| n.rotation).collect();
        let mut local_s: Vec<Vec3> = self.nodes.iter().map(|n| n.scale).collect();

        // Sample animated channels
        for ch in &anim.channels {
            if ch.timestamps.is_empty()
                || ch.values.is_empty()
                || ch.target_node >= self.nodes.len()
            {
                continue;
            }

            let val = if time <= ch.timestamps[0] {
                ch.values[0]
            } else if time >= *ch.timestamps.last().unwrap() {
                *ch.values.last().unwrap()
            } else {
                let mut idx = 0;
                while idx + 1 < ch.timestamps.len() && ch.timestamps[idx + 1] <= time {
                    idx += 1;
                }
                let t0 = ch.timestamps[idx];
                let t1 = ch.timestamps[idx + 1];
                let alpha = if (t1 - t0).abs() > 1e-6 {
                    (time - t0) / (t1 - t0)
                } else {
                    0.0
                };
                let v0 = ch.values[idx];
                let v1 = ch.values[idx + 1];

                match ch.property {
                    AnimationProperty::Rotation => {
                        let q0 = Quat::new(v0[0], v0[1], v0[2], v0[3]);
                        let q1 = Quat::new(v1[0], v1[1], v1[2], v1[3]);
                        let q = q0.nlerp(q1, alpha);
                        [q.x, q.y, q.z, q.w]
                    }
                    AnimationProperty::Translation | AnimationProperty::Scale => [
                        v0[0] + (v1[0] - v0[0]) * alpha,
                        v0[1] + (v1[1] - v0[1]) * alpha,
                        v0[2] + (v1[2] - v0[2]) * alpha,
                        1.0,
                    ],
                }
            };

            match ch.property {
                AnimationProperty::Translation => {
                    local_t[ch.target_node] = Vec3::new(val[0], val[1], val[2]);
                }
                AnimationProperty::Rotation => {
                    local_r[ch.target_node] = Quat::new(val[0], val[1], val[2], val[3]).normalize();
                }
                AnimationProperty::Scale => {
                    local_s[ch.target_node] = Vec3::new(val[0], val[1], val[2]);
                }
            }
        }

        // Forward Kinematics (FK)
        let mut world_matrices = vec![Mat4::IDENTITY; self.nodes.len()];
        let mut is_child = vec![false; self.nodes.len()];
        for node in &self.nodes {
            for &child in &node.children {
                if child < is_child.len() {
                    is_child[child] = true;
                }
            }
        }

        fn eval_fk(
            node_idx: usize,
            parent_world: &Mat4,
            nodes: &[GltfNodeData],
            local_t: &[Vec3],
            local_r: &[Quat],
            local_s: &[Vec3],
            world_matrices: &mut [Mat4],
        ) {
            let local_m = Mat4::from_translation_rotation_scale(
                local_t[node_idx],
                local_r[node_idx],
                local_s[node_idx],
            );
            let world_m = parent_world.mul(&local_m);
            world_matrices[node_idx] = world_m;

            for &child in &nodes[node_idx].children {
                if child < nodes.len() {
                    eval_fk(
                        child,
                        &world_m,
                        nodes,
                        local_t,
                        local_r,
                        local_s,
                        world_matrices,
                    );
                }
            }
        }

        for (i, &child) in is_child.iter().enumerate() {
            if !child {
                eval_fk(
                    i,
                    &Mat4::IDENTITY,
                    &self.nodes,
                    &local_t,
                    &local_r,
                    &local_s,
                    &mut world_matrices,
                );
            }
        }

        // Joint skinning matrices: S_j = M_joint_world * IBM_j
        let mut skin_matrices = Vec::with_capacity(skin.joints.len());
        for (j, &joint_node) in skin.joints.iter().enumerate() {
            let ibm = skin
                .inverse_bind_matrices
                .get(j)
                .copied()
                .unwrap_or(Mat4::IDENTITY);
            let world = world_matrices
                .get(joint_node)
                .copied()
                .unwrap_or(Mat4::IDENTITY);
            skin_matrices.push(world.mul(&ibm));
        }

        // Vertex Skinning
        let mut deformed = Vec::with_capacity(self.skinned_vertices.len());
        for sv in &self.skinned_vertices {
            let mut pos = Vec3::new(0.0, 0.0, 0.0);
            for i in 0..4 {
                let w = sv.weights[i];
                if w > 1e-4 {
                    let j_idx = sv.joints[i] as usize;
                    if let Some(m) = skin_matrices.get(j_idx) {
                        let tp = m.transform_point(sv.position);
                        pos.x += tp.x * w;
                        pos.y += tp.y * w;
                        pos.z += tp.z * w;
                    }
                }
            }
            deformed.push(pos);
        }

        let mut mesh = Mesh3D::new(deformed, self.mesh.faces.clone());
        apply_normalization(&mut mesh, self.rest_center, self.rest_scale);
        mesh
    }
}

/// Load a binary `.glb` model from raw bytes.
pub fn parse_glb(bytes: &[u8]) -> Result<GltfModel, String> {
    if bytes.len() < 12 {
        return Err("Invalid GLB: file too short".into());
    }

    let magic = u32::from_le_bytes(bytes[0..4].try_into().unwrap());
    if magic != 0x4654_6C67 {
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
                let s = std::str::from_utf8(chunk_data)
                    .map_err(|e| format!("Invalid UTF-8 in GLB JSON chunk: {e}"))?;
                json_str = Some(s);
            }
            0x004E_4942 => {
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
    let float_bytes = 12;
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

fn read_mat4_accessor(
    root: &GltfRoot,
    buffers: &[Vec<u8>],
    accessor_idx: usize,
) -> Result<Vec<Mat4>, String> {
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
    let matrix_bytes = 64; // 16 * 4
    if start + accessor.count * matrix_bytes > buf.len() {
        return Err("Buffer overrun reading MAT4".into());
    }

    let mut matrices = Vec::with_capacity(accessor.count);
    for i in 0..accessor.count {
        let o = start + i * matrix_bytes;
        let mut m = [0.0f32; 16];
        for (k, item) in m.iter_mut().enumerate() {
            let mo = o + k * 4;
            *item = f32::from_le_bytes(buf[mo..mo + 4].try_into().unwrap());
        }
        matrices.push(Mat4(m));
    }

    Ok(matrices)
}

fn read_vec4_u16_accessor(
    root: &GltfRoot,
    buffers: &[Vec<u8>],
    accessor_idx: usize,
) -> Result<Vec<[u16; 4]>, String> {
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
    let mut out = Vec::with_capacity(accessor.count);

    match accessor.component_type {
        5121 => {
            let item_size = 4;
            if start + accessor.count * item_size > buf.len() {
                return Err("Buffer overrun reading u8 JOINTS_0".into());
            }
            for i in 0..accessor.count {
                let o = start + i * item_size;
                out.push([
                    buf[o] as u16,
                    buf[o + 1] as u16,
                    buf[o + 2] as u16,
                    buf[o + 3] as u16,
                ]);
            }
        }
        5123 => {
            let item_size = 8;
            if start + accessor.count * item_size > buf.len() {
                return Err("Buffer overrun reading u16 JOINTS_0".into());
            }
            for i in 0..accessor.count {
                let o = start + i * item_size;
                out.push([
                    u16::from_le_bytes(buf[o..o + 2].try_into().unwrap()),
                    u16::from_le_bytes(buf[o + 2..o + 4].try_into().unwrap()),
                    u16::from_le_bytes(buf[o + 4..o + 6].try_into().unwrap()),
                    u16::from_le_bytes(buf[o + 6..o + 8].try_into().unwrap()),
                ]);
            }
        }
        _ => return Err("Unsupported componentType for JOINTS_0".into()),
    }

    Ok(out)
}

fn read_vec4_f32_accessor(
    root: &GltfRoot,
    buffers: &[Vec<u8>],
    accessor_idx: usize,
) -> Result<Vec<[f32; 4]>, String> {
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
    let mut out = Vec::with_capacity(accessor.count);

    match accessor.component_type {
        5126 => {
            let item_size = 16;
            if start + accessor.count * item_size > buf.len() {
                return Err("Buffer overrun reading f32 WEIGHTS_0".into());
            }
            for i in 0..accessor.count {
                let o = start + i * item_size;
                out.push([
                    f32::from_le_bytes(buf[o..o + 4].try_into().unwrap()),
                    f32::from_le_bytes(buf[o + 4..o + 8].try_into().unwrap()),
                    f32::from_le_bytes(buf[o + 8..o + 12].try_into().unwrap()),
                    f32::from_le_bytes(buf[o + 12..o + 16].try_into().unwrap()),
                ]);
            }
        }
        5121 => {
            let item_size = 4;
            if start + accessor.count * item_size > buf.len() {
                return Err("Buffer overrun reading u8 WEIGHTS_0".into());
            }
            for i in 0..accessor.count {
                let o = start + i * item_size;
                out.push([
                    buf[o] as f32 / 255.0,
                    buf[o + 1] as f32 / 255.0,
                    buf[o + 2] as f32 / 255.0,
                    buf[o + 3] as f32 / 255.0,
                ]);
            }
        }
        _ => return Err("Unsupported componentType for WEIGHTS_0".into()),
    }

    Ok(out)
}

fn read_f32_accessor(
    root: &GltfRoot,
    buffers: &[Vec<u8>],
    accessor_idx: usize,
) -> Result<Vec<f32>, String> {
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
    let item_size = 4;
    if start + accessor.count * item_size > buf.len() {
        return Err("Buffer overrun reading f32 timeline".into());
    }

    let mut out = Vec::with_capacity(accessor.count);
    for i in 0..accessor.count {
        let o = start + i * item_size;
        out.push(f32::from_le_bytes(buf[o..o + 4].try_into().unwrap()));
    }

    Ok(out)
}

fn read_anim_values_accessor(
    root: &GltfRoot,
    buffers: &[Vec<u8>],
    accessor_idx: usize,
    property: AnimationProperty,
) -> Result<Vec<[f32; 4]>, String> {
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
    let mut out = Vec::with_capacity(accessor.count);

    match property {
        AnimationProperty::Rotation => {
            let item_size = 16;
            if start + accessor.count * item_size > buf.len() {
                return Err("Buffer overrun reading rotation values".into());
            }
            for i in 0..accessor.count {
                let o = start + i * item_size;
                out.push([
                    f32::from_le_bytes(buf[o..o + 4].try_into().unwrap()),
                    f32::from_le_bytes(buf[o + 4..o + 8].try_into().unwrap()),
                    f32::from_le_bytes(buf[o + 8..o + 12].try_into().unwrap()),
                    f32::from_le_bytes(buf[o + 12..o + 16].try_into().unwrap()),
                ]);
            }
        }
        AnimationProperty::Translation | AnimationProperty::Scale => {
            let item_size = 12;
            if start + accessor.count * item_size > buf.len() {
                return Err("Buffer overrun reading translation/scale values".into());
            }
            for i in 0..accessor.count {
                let o = start + i * item_size;
                out.push([
                    f32::from_le_bytes(buf[o..o + 4].try_into().unwrap()),
                    f32::from_le_bytes(buf[o + 4..o + 8].try_into().unwrap()),
                    f32::from_le_bytes(buf[o + 8..o + 12].try_into().unwrap()),
                    1.0,
                ]);
            }
        }
    }

    Ok(out)
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
        _ => return Err("Unsupported index componentType".into()),
    }

    let mut faces = Vec::with_capacity(flat_indices.len() / 3);
    for chunk in flat_indices.chunks_exact(3) {
        faces.push(vec![chunk[0], chunk[1], chunk[2]]);
    }

    Ok(faces)
}

fn compute_bounds(vertices: &[Vec3]) -> (Vec3, f32) {
    if vertices.is_empty() {
        return (Vec3::default(), 1.0);
    }

    let mut min_x = f32::MAX;
    let mut max_x = f32::MIN;
    let mut min_y = f32::MAX;
    let mut max_y = f32::MIN;
    let mut min_z = f32::MAX;
    let mut max_z = f32::MIN;

    for v in vertices {
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

    (Vec3::new(cx, cy, cz), 1.0 / max_span)
}

fn apply_normalization(mesh: &mut Mesh3D, center: Vec3, scale: f32) {
    for v in &mut mesh.vertices {
        v.x = (v.x - center.x) * scale;
        v.y = (v.y - center.y) * scale;
        v.z = (v.z - center.z) * scale;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_embedded_gltf_triangle() {
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

    #[test]
    fn test_skeletal_bone_animation_pose_sampling() {
        let mut bin = Vec::new();

        // Accessor 0: 3 Positions [(-1, 0, 0), (1, 0, 0), (0, 1, 0)]
        let pos_start = bin.len();
        for &(x, y, z) in &[
            (-1.0f32, 0.0f32, 0.0f32),
            (1.0f32, 0.0f32, 0.0f32),
            (0.0f32, 1.0f32, 0.0f32),
        ] {
            bin.extend_from_slice(&x.to_le_bytes());
            bin.extend_from_slice(&y.to_le_bytes());
            bin.extend_from_slice(&z.to_le_bytes());
        }
        let pos_len = bin.len() - pos_start;

        // Accessor 1: JOINTS_0 (u16 x 4) -> all bound to joint 1
        let joints_start = bin.len();
        for _ in 0..3 {
            bin.extend_from_slice(&1u16.to_le_bytes());
            bin.extend_from_slice(&0u16.to_le_bytes());
            bin.extend_from_slice(&0u16.to_le_bytes());
            bin.extend_from_slice(&0u16.to_le_bytes());
        }
        let joints_len = bin.len() - joints_start;

        // Accessor 2: WEIGHTS_0 (f32 x 4) -> 1.0 weight on joint 1
        let weights_start = bin.len();
        for _ in 0..3 {
            bin.extend_from_slice(&1.0f32.to_le_bytes());
            bin.extend_from_slice(&0.0f32.to_le_bytes());
            bin.extend_from_slice(&0.0f32.to_le_bytes());
            bin.extend_from_slice(&0.0f32.to_le_bytes());
        }
        let weights_len = bin.len() - weights_start;

        // Accessor 3: Inverse bind matrices (2 x Mat4 = 2 x 64 = 128 bytes)
        let ibm_start = bin.len();
        for _ in 0..2 {
            for v in Mat4::IDENTITY.0 {
                bin.extend_from_slice(&v.to_le_bytes());
            }
        }
        let ibm_len = bin.len() - ibm_start;

        // Accessor 4: Animation timestamps [0.0, 1.0]
        let time_start = bin.len();
        bin.extend_from_slice(&0.0f32.to_le_bytes());
        bin.extend_from_slice(&1.0f32.to_le_bytes());
        let time_len = bin.len() - time_start;

        // Accessor 5: Animation rotations (2 x Quat: identity at t=0, 90 deg z-rotation at t=1)
        let rot_start = bin.len();
        // t=0: Quat::IDENTITY = [0, 0, 0, 1]
        bin.extend_from_slice(&0.0f32.to_le_bytes());
        bin.extend_from_slice(&0.0f32.to_le_bytes());
        bin.extend_from_slice(&0.0f32.to_le_bytes());
        bin.extend_from_slice(&1.0f32.to_le_bytes());
        // t=1: 90 deg z rot: sin(pi/4) = 0.7071068, cos(pi/4) = 0.7071068
        let half_angle = std::f32::consts::FRAC_PI_4;
        let s = half_angle.sin();
        let c = half_angle.cos();
        bin.extend_from_slice(&0.0f32.to_le_bytes());
        bin.extend_from_slice(&0.0f32.to_le_bytes());
        bin.extend_from_slice(&s.to_le_bytes());
        bin.extend_from_slice(&c.to_le_bytes());
        let rot_len = bin.len() - rot_start;

        let b64 = base64::engine::general_purpose::STANDARD.encode(&bin);

        let json = format!(
            r#"{{
                "asset": {{ "version": "2.0" }},
                "buffers": [{{ "byteLength": {}, "uri": "data:application/octet-stream;base64,{}" }}],
                "bufferViews": [
                    {{ "buffer": 0, "byteOffset": {}, "byteLength": {} }},
                    {{ "buffer": 0, "byteOffset": {}, "byteLength": {} }},
                    {{ "buffer": 0, "byteOffset": {}, "byteLength": {} }},
                    {{ "buffer": 0, "byteOffset": {}, "byteLength": {} }},
                    {{ "buffer": 0, "byteOffset": {}, "byteLength": {} }},
                    {{ "buffer": 0, "byteOffset": {}, "byteLength": {} }}
                ],
                "accessors": [
                    {{ "bufferView": 0, "byteOffset": 0, "componentType": 5126, "count": 3, "type": "VEC3" }},
                    {{ "bufferView": 1, "byteOffset": 0, "componentType": 5123, "count": 3, "type": "VEC4" }},
                    {{ "bufferView": 2, "byteOffset": 0, "componentType": 5126, "count": 3, "type": "VEC4" }},
                    {{ "bufferView": 3, "byteOffset": 0, "componentType": 5126, "count": 2, "type": "MAT4" }},
                    {{ "bufferView": 4, "byteOffset": 0, "componentType": 5126, "count": 2, "type": "SCALAR" }},
                    {{ "bufferView": 5, "byteOffset": 0, "componentType": 5126, "count": 2, "type": "VEC4" }}
                ],
                "nodes": [
                    {{ "name": "RootBone", "children": [1] }},
                    {{ "name": "AnimatedBone" }}
                ],
                "skins": [
                    {{
                        "name": "Armature",
                        "inverseBindMatrices": 3,
                        "joints": [0, 1]
                    }}
                ],
                "animations": [
                    {{
                        "name": "BoneDance",
                        "channels": [
                            {{ "sampler": 0, "target": {{ "node": 1, "path": "rotation" }} }}
                        ],
                        "samplers": [
                            {{ "input": 4, "output": 5 }}
                        ]
                    }}
                ],
                "meshes": [{{
                    "name": "SkinnedMesh",
                    "primitives": [{{
                        "attributes": {{
                            "POSITION": 0,
                            "JOINTS_0": 1,
                            "WEIGHTS_0": 2
                        }}
                    }}]
                }}]
            }}"#,
            bin.len(),
            b64,
            pos_start,
            pos_len,
            joints_start,
            joints_len,
            weights_start,
            weights_len,
            ibm_start,
            ibm_len,
            time_start,
            time_len,
            rot_start,
            rot_len
        );

        let model = parse_gltf(&json, None).expect("parse skinned gltf");
        assert_eq!(model.name, "SkinnedMesh");
        assert!(model.skin.is_some());
        assert_eq!(model.animations.len(), 1);

        // Sample pose at t=0.0 (rest pose)
        let mesh_t0 = model.sample_pose(0.0);
        // Sample pose at t=1.0 (90 deg rotation)
        let mesh_t1 = model.sample_pose(1.0);

        // At t=1.0, the vertices should be rotated
        assert_ne!(
            mesh_t0.vertices[0], mesh_t1.vertices[0],
            "Skeletal animation at t=1.0 must deform vertex positions"
        );
    }
}
