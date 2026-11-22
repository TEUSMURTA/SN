use std::io::Write;

use serde_json::{Value, json};

use crate::config::Limits;
use crate::error::{Result, SnError};
use crate::glb::{padded_len, write_glb_container};
use crate::model::{ColorSpace, ConversionReport, Material, PreparedAsset, PreparedTexture};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GlbDocument {
    pub json: Vec<u8>,
    pub binary: Vec<u8>,
    pub report: ConversionReport,
}

#[derive(Debug, Default)]
struct BinBuilder {
    bytes: Vec<u8>,
}

impl BinBuilder {
    fn new() -> Self {
        Self { bytes: Vec::new() }
    }

    fn align_to_4(&mut self) -> Result<()> {
        let rem = self.bytes.len() % 4;
        if rem != 0 {
            let pad = 4 - rem;
            self.bytes
                .try_reserve(pad)
                .map_err(|_| SnError::AllocationFailed {
                    resource: "GLB binary alignment padding",
                })?;
            self.bytes.resize(self.bytes.len() + pad, 0);
        }
        Ok(())
    }

    fn reserve_chunk(&mut self, len: usize, resource: &'static str) -> Result<usize> {
        self.align_to_4()?;
        let offset = self.bytes.len();
        offset.checked_add(len).ok_or(SnError::SizeOverflow {
            resource: "GLB binary buffer",
        })?;
        self.bytes
            .try_reserve(len)
            .map_err(|_| SnError::AllocationFailed { resource })?;
        Ok(offset)
    }

    fn finish_chunk(&mut self, offset: usize, len: usize) -> Result<(usize, usize)> {
        debug_assert_eq!(self.bytes.len(), offset + len);
        self.align_to_4()?;
        Ok((offset, len))
    }

    fn push_slice(&mut self, data: &[u8]) -> Result<(usize, usize)> {
        let offset = self.reserve_chunk(data.len(), "GLB binary chunk")?;
        self.bytes.extend_from_slice(data);
        self.finish_chunk(offset, data.len())
    }

    fn push_f32_2(&mut self, values: &[[f32; 2]]) -> Result<(usize, usize)> {
        let item_bytes = 2_usize
            .checked_mul(4)
            .ok_or(SnError::SizeOverflow { resource: "vec2" })?;
        let byte_len = values
            .len()
            .checked_mul(item_bytes)
            .ok_or(SnError::SizeOverflow {
                resource: "GLB vec2 accessor",
            })?;
        let offset = self.reserve_chunk(byte_len, "GLB vec2 buffer")?;
        for v in values {
            self.bytes.extend_from_slice(&v[0].to_le_bytes());
            self.bytes.extend_from_slice(&v[1].to_le_bytes());
        }
        self.finish_chunk(offset, byte_len)
    }

    fn push_f32_3(&mut self, values: &[[f32; 3]]) -> Result<(usize, usize)> {
        let item_bytes = 3_usize
            .checked_mul(4)
            .ok_or(SnError::SizeOverflow { resource: "vec3" })?;
        let byte_len = values
            .len()
            .checked_mul(item_bytes)
            .ok_or(SnError::SizeOverflow {
                resource: "GLB vec3 accessor",
            })?;
        let offset = self.reserve_chunk(byte_len, "GLB vec3 buffer")?;
        for v in values {
            self.bytes.extend_from_slice(&v[0].to_le_bytes());
            self.bytes.extend_from_slice(&v[1].to_le_bytes());
            self.bytes.extend_from_slice(&v[2].to_le_bytes());
        }
        self.finish_chunk(offset, byte_len)
    }

    fn push_f32_4(&mut self, values: &[[f32; 4]]) -> Result<(usize, usize)> {
        let item_bytes = 4_usize
            .checked_mul(4)
            .ok_or(SnError::SizeOverflow { resource: "vec4" })?;
        let byte_len = values
            .len()
            .checked_mul(item_bytes)
            .ok_or(SnError::SizeOverflow {
                resource: "GLB vec4 accessor",
            })?;
        let offset = self.reserve_chunk(byte_len, "GLB vec4 buffer")?;
        for v in values {
            self.bytes.extend_from_slice(&v[0].to_le_bytes());
            self.bytes.extend_from_slice(&v[1].to_le_bytes());
            self.bytes.extend_from_slice(&v[2].to_le_bytes());
            self.bytes.extend_from_slice(&v[3].to_le_bytes());
        }
        self.finish_chunk(offset, byte_len)
    }

    fn push_u16_4(&mut self, values: &[[u16; 4]]) -> Result<(usize, usize)> {
        let item_bytes = 4_usize
            .checked_mul(2)
            .ok_or(SnError::SizeOverflow { resource: "u16x4" })?;
        let byte_len = values
            .len()
            .checked_mul(item_bytes)
            .ok_or(SnError::SizeOverflow {
                resource: "GLB joints accessor",
            })?;
        let offset = self.reserve_chunk(byte_len, "GLB joints buffer")?;
        for v in values {
            self.bytes.extend_from_slice(&v[0].to_le_bytes());
            self.bytes.extend_from_slice(&v[1].to_le_bytes());
            self.bytes.extend_from_slice(&v[2].to_le_bytes());
            self.bytes.extend_from_slice(&v[3].to_le_bytes());
        }
        self.finish_chunk(offset, byte_len)
    }

    fn push_indices(&mut self, values: &[u32]) -> Result<(usize, usize, u32)> {
        let (component_type, width) = index_format(values)?;
        let byte_len = values
            .len()
            .checked_mul(width)
            .ok_or(SnError::SizeOverflow {
                resource: "GLB index accessor",
            })?;
        let offset = self.reserve_chunk(byte_len, "GLB index buffer")?;
        for &value in values {
            let bytes = value.to_le_bytes();
            self.bytes.extend_from_slice(&bytes[..width]);
        }
        let (offset, len) = self.finish_chunk(offset, byte_len)?;
        Ok((offset, len, component_type))
    }
}

fn index_format(values: &[u32]) -> Result<(u32, usize)> {
    let max_index = values
        .iter()
        .copied()
        .max()
        .ok_or_else(|| SnError::InvalidData {
            resource: "indices",
            reason: "index accessor cannot be empty".to_owned(),
        })?;
    let component_type = if max_index <= u32::from(u8::MAX) {
        (5121, 1)
    } else if max_index <= u32::from(u16::MAX) {
        (5123, 2)
    } else {
        (5125, 4)
    };
    Ok(component_type)
}

fn add_binary_values(total: &mut u64, count: usize, item_bytes: usize) -> Result<()> {
    let byte_len = count.checked_mul(item_bytes).ok_or(SnError::SizeOverflow {
        resource: "GLB binary buffer",
    })?;
    let byte_len = u64::try_from(byte_len).map_err(|_| SnError::SizeOverflow {
        resource: "GLB binary buffer",
    })?;
    let padded = padded_len(byte_len, "GLB binary buffer")?;
    *total = total.checked_add(padded).ok_or(SnError::SizeOverflow {
        resource: "GLB binary buffer",
    })?;
    Ok(())
}

/// Rejects an oversized binary payload before copying textures or vertex data.
/// JSON and container overhead are checked exactly when the GLB layout is written.
fn validate_binary_budget(asset: &PreparedAsset, limits: &Limits) -> Result<()> {
    let mut binary_len = 0_u64;
    for texture in &asset.textures {
        add_binary_values(&mut binary_len, texture.png_bytes.len(), 1)?;
    }
    for primitive in &asset.mesh.primitives {
        let attributes = &primitive.attributes;
        add_binary_values(&mut binary_len, attributes.positions.len(), 12)?;
        if let Some(values) = &attributes.normals {
            add_binary_values(&mut binary_len, values.len(), 12)?;
        }
        if let Some(values) = &attributes.tangents {
            add_binary_values(&mut binary_len, values.len(), 16)?;
        }
        if let Some(values) = &attributes.texcoords {
            add_binary_values(&mut binary_len, values.len(), 8)?;
        }
        if let Some(values) = &attributes.joints {
            add_binary_values(&mut binary_len, values.len(), 8)?;
        }
        if let Some(values) = &attributes.weights {
            add_binary_values(&mut binary_len, values.len(), 16)?;
        }
        let (_, index_width) = index_format(&primitive.indices)?;
        add_binary_values(&mut binary_len, primitive.indices.len(), index_width)?;
    }

    if binary_len > limits.max_asset_bytes {
        return Err(SnError::LimitExceeded {
            resource: "GLB binary buffer",
            actual: binary_len,
            limit: limits.max_asset_bytes,
        });
    }
    Ok(())
}

fn validate_aggregate_limits(asset: &PreparedAsset, limits: &Limits) -> Result<(u64, u64, u64)> {
    let mut total_vertices: u64 = 0;
    let mut total_indices: u64 = 0;
    let mut total_texture_bytes: u64 = 0;

    let max_vertices = u64::try_from(limits.max_vertices).map_err(|_| SnError::SizeOverflow {
        resource: "vertices limit",
    })?;
    let max_indices = u64::try_from(limits.max_indices).map_err(|_| SnError::SizeOverflow {
        resource: "indices limit",
    })?;

    for prim in &asset.mesh.primitives {
        let v_count =
            u64::try_from(prim.attributes.positions.len()).map_err(|_| SnError::SizeOverflow {
                resource: "vertices",
            })?;
        total_vertices = total_vertices
            .checked_add(v_count)
            .ok_or(SnError::SizeOverflow {
                resource: "total vertices",
            })?;
        if total_vertices > max_vertices {
            return Err(SnError::LimitExceeded {
                resource: "vertices",
                actual: total_vertices,
                limit: max_vertices,
            });
        }

        let i_count = u64::try_from(prim.indices.len()).map_err(|_| SnError::SizeOverflow {
            resource: "indices",
        })?;
        total_indices = total_indices
            .checked_add(i_count)
            .ok_or(SnError::SizeOverflow {
                resource: "total indices",
            })?;
        if total_indices > max_indices {
            return Err(SnError::LimitExceeded {
                resource: "indices",
                actual: total_indices,
                limit: max_indices,
            });
        }
    }

    for tex in &asset.textures {
        let t_bytes = u64::try_from(tex.png_bytes.len()).map_err(|_| SnError::SizeOverflow {
            resource: "texture bytes",
        })?;
        total_texture_bytes =
            total_texture_bytes
                .checked_add(t_bytes)
                .ok_or(SnError::SizeOverflow {
                    resource: "total texture bytes",
                })?;
        if total_texture_bytes > limits.max_texture_bytes {
            return Err(SnError::LimitExceeded {
                resource: "texture bytes",
                actual: total_texture_bytes,
                limit: limits.max_texture_bytes,
            });
        }
    }

    Ok((total_vertices, total_indices, total_texture_bytes))
}

fn validate_asset(asset: &PreparedAsset) -> Result<()> {
    if asset.mesh.primitives.is_empty() {
        return Err(SnError::InvalidData {
            resource: "mesh",
            reason: "at least one primitive is required".to_owned(),
        });
    }

    for primitive in &asset.mesh.primitives {
        let attributes = &primitive.attributes;
        let vertex_count = attributes.positions.len();
        if vertex_count == 0 {
            return Err(SnError::InvalidData {
                resource: "positions",
                reason: "POSITION cannot be empty".to_owned(),
            });
        }
        if primitive.indices.is_empty() || !primitive.indices.len().is_multiple_of(3) {
            return Err(SnError::InvalidData {
                resource: "indices",
                reason: "indices must contain complete, non-empty triangles".to_owned(),
            });
        }
        for &index in &primitive.indices {
            let index = usize::try_from(index).map_err(|_| SnError::InvalidData {
                resource: "indices",
                reason: "index does not fit in usize".to_owned(),
            })?;
            if index >= vertex_count {
                return Err(SnError::InvalidData {
                    resource: "indices",
                    reason: format!("index {index} is outside a vertex count of {vertex_count}"),
                });
            }
        }
        if let Some(material) = primitive.material
            && material >= asset.materials.len()
        {
            return Err(SnError::InvalidData {
                resource: "mesh materials",
                reason: format!(
                    "primitive references material {material}, but only {} materials exist",
                    asset.materials.len()
                ),
            });
        }

        check_attribute_len(
            "normals",
            attributes.normals.as_ref().map(Vec::len),
            vertex_count,
        )?;
        check_attribute_len(
            "tangents",
            attributes.tangents.as_ref().map(Vec::len),
            vertex_count,
        )?;
        check_attribute_len(
            "texture coordinates",
            attributes.texcoords.as_ref().map(Vec::len),
            vertex_count,
        )?;
        check_attribute_len(
            "joints",
            attributes.joints.as_ref().map(Vec::len),
            vertex_count,
        )?;
        check_attribute_len(
            "weights",
            attributes.weights.as_ref().map(Vec::len),
            vertex_count,
        )?;
        ensure_finite(&attributes.positions, "positions")?;
        if let Some(values) = &attributes.normals {
            ensure_finite(values, "normals")?;
        }
        if let Some(values) = &attributes.tangents {
            ensure_finite(values, "tangents")?;
        }
        if let Some(values) = &attributes.texcoords {
            ensure_finite(values, "texture coordinates")?;
        }
        if let Some(values) = &attributes.weights {
            ensure_finite(values, "weights")?;
        }
    }

    for material in &asset.materials {
        for texture in [
            material.base_color_texture,
            material.normal_texture,
            material.metallic_roughness_texture,
        ]
        .into_iter()
        .flatten()
        {
            if texture >= asset.textures.len() {
                return Err(SnError::InvalidData {
                    resource: "material textures",
                    reason: format!(
                        "material references texture {texture}, but only {} textures exist",
                        asset.textures.len()
                    ),
                });
            }
        }
        ensure_finite(&[material.base_color_factor], "base color factor")?;
        if !(0.0..=1.0).contains(&material.metallic_factor)
            || !(0.0..=1.0).contains(&material.roughness_factor)
        {
            return Err(SnError::InvalidData {
                resource: "material factors",
                reason: "metallic and roughness factors must be finite and between 0 and 1"
                    .to_owned(),
            });
        }
    }
    Ok(())
}

fn check_attribute_len(
    resource: &'static str,
    actual: Option<usize>,
    expected: usize,
) -> Result<()> {
    if let Some(actual) = actual
        && actual != expected
    {
        return Err(SnError::InvalidData {
            resource,
            reason: format!("attribute has {actual} values, expected {expected}"),
        });
    }
    Ok(())
}

fn ensure_finite<const N: usize>(values: &[[f32; N]], resource: &'static str) -> Result<()> {
    if values.iter().flatten().any(|value| !value.is_finite()) {
        return Err(SnError::InvalidData {
            resource,
            reason: "floating-point values must be finite".to_owned(),
        });
    }
    Ok(())
}

fn vec3_min_max(coords: &[[f32; 3]]) -> Result<(Vec<f32>, Vec<f32>)> {
    if coords.is_empty() {
        return Err(SnError::InvalidData {
            resource: "positions",
            reason: "primitive has no positions".to_owned(),
        });
    }
    let mut min = [f32::INFINITY; 3];
    let mut max = [f32::NEG_INFINITY; 3];
    for p in coords {
        for i in 0..3 {
            if p[i] < min[i] {
                min[i] = p[i];
            }
            if p[i] > max[i] {
                max[i] = p[i];
            }
        }
    }
    Ok((vec![min[0], min[1], min[2]], vec![max[0], max[1], max[2]]))
}

fn build_material_json(mat: &Material, index: usize) -> Value {
    let name = if mat.name.is_empty() {
        format!("material_{index}")
    } else {
        mat.name.clone()
    };

    let mut pbr = serde_json::Map::new();
    pbr.insert("baseColorFactor".to_owned(), json!(mat.base_color_factor));
    pbr.insert("metallicFactor".to_owned(), json!(mat.metallic_factor));
    pbr.insert("roughnessFactor".to_owned(), json!(mat.roughness_factor));

    if let Some(tex_idx) = mat.base_color_texture {
        pbr.insert("baseColorTexture".to_owned(), json!({ "index": tex_idx }));
    }
    if let Some(mr_idx) = mat.metallic_roughness_texture {
        pbr.insert(
            "metallicRoughnessTexture".to_owned(),
            json!({ "index": mr_idx }),
        );
    }

    let mut obj = serde_json::Map::new();
    obj.insert("name".to_owned(), json!(name));
    obj.insert("pbrMetallicRoughness".to_owned(), Value::Object(pbr));

    if let Some(nrm_idx) = mat.normal_texture {
        obj.insert("normalTexture".to_owned(), json!({ "index": nrm_idx }));
    }

    Value::Object(obj)
}

fn push_textures_to_bin(
    textures: &[PreparedTexture],
    bin: &mut BinBuilder,
    buffer_views: &mut Vec<Value>,
    images: &mut Vec<Value>,
    gltf_textures: &mut Vec<Value>,
) -> Result<()> {
    for (tex_idx, tex) in textures.iter().enumerate() {
        let (offset, len) = bin.push_slice(&tex.png_bytes)?;
        let bv_idx = buffer_views.len();
        buffer_views.push(json!({
            "buffer": 0,
            "byteOffset": offset,
            "byteLength": len
        }));

        let img_name = if tex.name.is_empty() {
            format!("image_{tex_idx}")
        } else {
            tex.name.clone()
        };
        images.push(json!({
            "name": img_name,
            "mimeType": "image/png",
            "bufferView": bv_idx
        }));

        let tex_name = if tex.name.is_empty() {
            format!("texture_{tex_idx}")
        } else {
            tex.name.clone()
        };
        let mut tex_obj = serde_json::Map::new();
        tex_obj.insert("name".to_owned(), json!(tex_name));
        tex_obj.insert("source".to_owned(), json!(tex_idx));
        if matches!(tex.color_space, ColorSpace::Srgb | ColorSpace::Linear) {
            gltf_textures.push(Value::Object(tex_obj));
        }
    }
    Ok(())
}

pub fn build_glb_document(asset: &PreparedAsset, limits: &Limits) -> Result<GlbDocument> {
    validate_asset(asset)?;
    let (total_vertices, total_indices, total_texture_bytes) =
        validate_aggregate_limits(asset, limits)?;
    validate_binary_budget(asset, limits)?;

    let mut bin = BinBuilder::new();
    let mut buffer_views: Vec<Value> = Vec::new();
    let mut accessors: Vec<Value> = Vec::new();
    let mut images: Vec<Value> = Vec::new();
    let mut gltf_textures: Vec<Value> = Vec::new();

    push_textures_to_bin(
        &asset.textures,
        &mut bin,
        &mut buffer_views,
        &mut images,
        &mut gltf_textures,
    )?;

    let mut gltf_materials: Vec<Value> = Vec::new();
    if !asset.materials.is_empty() {
        for (idx, mat) in asset.materials.iter().enumerate() {
            gltf_materials.push(build_material_json(mat, idx));
        }
    } else if !asset.mesh.primitives.is_empty() {
        let fallback = Material {
            name: "default_material".to_owned(),
            ..Material::default()
        };
        gltf_materials.push(build_material_json(&fallback, 0));
    }

    let mut primitives_json: Vec<Value> = Vec::new();
    for prim in &asset.mesh.primitives {
        let (min_pos, max_pos) = vec3_min_max(&prim.attributes.positions)?;
        let (pos_off, pos_len) = bin.push_f32_3(&prim.attributes.positions)?;
        let pos_bv = buffer_views.len();
        buffer_views.push(json!({
            "buffer": 0,
            "byteOffset": pos_off,
            "byteLength": pos_len,
            "target": 34962
        }));
        let pos_acc = accessors.len();
        accessors.push(json!({
            "bufferView": pos_bv,
            "componentType": 5126,
            "count": prim.attributes.positions.len(),
            "type": "VEC3",
            "min": min_pos,
            "max": max_pos
        }));

        let mut attrs = serde_json::Map::new();
        attrs.insert("POSITION".to_owned(), json!(pos_acc));

        if let Some(normals) = &prim.attributes.normals {
            if normals.len() != prim.attributes.positions.len() {
                return Err(SnError::InvalidData {
                    resource: "normals",
                    reason: "normals count must match positions count".to_owned(),
                });
            }
            let (nrm_off, nrm_len) = bin.push_f32_3(normals)?;
            let nrm_bv = buffer_views.len();
            buffer_views.push(json!({
                "buffer": 0,
                "byteOffset": nrm_off,
                "byteLength": nrm_len,
                "target": 34962
            }));
            let nrm_acc = accessors.len();
            accessors.push(json!({
                "bufferView": nrm_bv,
                "componentType": 5126,
                "count": normals.len(),
                "type": "VEC3"
            }));
            attrs.insert("NORMAL".to_owned(), json!(nrm_acc));
        }

        if let Some(tangents) = &prim.attributes.tangents {
            if tangents.len() != prim.attributes.positions.len() {
                return Err(SnError::InvalidData {
                    resource: "tangents",
                    reason: "tangents count must match positions count".to_owned(),
                });
            }
            let (tan_off, tan_len) = bin.push_f32_4(tangents)?;
            let tan_bv = buffer_views.len();
            buffer_views.push(json!({
                "buffer": 0,
                "byteOffset": tan_off,
                "byteLength": tan_len,
                "target": 34962
            }));
            let tan_acc = accessors.len();
            accessors.push(json!({
                "bufferView": tan_bv,
                "componentType": 5126,
                "count": tangents.len(),
                "type": "VEC4"
            }));
            attrs.insert("TANGENT".to_owned(), json!(tan_acc));
        }

        if let Some(texcoords) = &prim.attributes.texcoords {
            if texcoords.len() != prim.attributes.positions.len() {
                return Err(SnError::InvalidData {
                    resource: "texcoords",
                    reason: "texcoords count must match positions count".to_owned(),
                });
            }
            let (uv_off, uv_len) = bin.push_f32_2(texcoords)?;
            let uv_bv = buffer_views.len();
            buffer_views.push(json!({
                "buffer": 0,
                "byteOffset": uv_off,
                "byteLength": uv_len,
                "target": 34962
            }));
            let uv_acc = accessors.len();
            accessors.push(json!({
                "bufferView": uv_bv,
                "componentType": 5126,
                "count": texcoords.len(),
                "type": "VEC2"
            }));
            attrs.insert("TEXCOORD_0".to_owned(), json!(uv_acc));
        }

        if let Some(joints) = &prim.attributes.joints {
            if joints.len() != prim.attributes.positions.len() {
                return Err(SnError::InvalidData {
                    resource: "joints",
                    reason: "joints count must match positions count".to_owned(),
                });
            }
            let (j_off, j_len) = bin.push_u16_4(joints)?;
            let j_bv = buffer_views.len();
            buffer_views.push(json!({
                "buffer": 0,
                "byteOffset": j_off,
                "byteLength": j_len,
                "target": 34962
            }));
            let j_acc = accessors.len();
            accessors.push(json!({
                "bufferView": j_bv,
                "componentType": 5123,
                "count": joints.len(),
                "type": "VEC4"
            }));
            attrs.insert("JOINTS_0".to_owned(), json!(j_acc));
        }

        if let Some(weights) = &prim.attributes.weights {
            if weights.len() != prim.attributes.positions.len() {
                return Err(SnError::InvalidData {
                    resource: "weights",
                    reason: "weights count must match positions count".to_owned(),
                });
            }
            let (w_off, w_len) = bin.push_f32_4(weights)?;
            let w_bv = buffer_views.len();
            buffer_views.push(json!({
                "buffer": 0,
                "byteOffset": w_off,
                "byteLength": w_len,
                "target": 34962
            }));
            let w_acc = accessors.len();
            accessors.push(json!({
                "bufferView": w_bv,
                "componentType": 5126,
                "count": weights.len(),
                "type": "VEC4"
            }));
            attrs.insert("WEIGHTS_0".to_owned(), json!(w_acc));
        }

        let mut prim_obj = serde_json::Map::new();
        prim_obj.insert("attributes".to_owned(), Value::Object(attrs));

        let (idx_off, idx_len, index_component_type) = bin.push_indices(&prim.indices)?;
        let idx_bv = buffer_views.len();
        buffer_views.push(json!({
            "buffer": 0,
            "byteOffset": idx_off,
            "byteLength": idx_len,
            "target": 34963
        }));
        let idx_acc = accessors.len();
        accessors.push(json!({
            "bufferView": idx_bv,
            "componentType": index_component_type,
            "count": prim.indices.len(),
            "type": "SCALAR"
        }));
        prim_obj.insert("indices".to_owned(), json!(idx_acc));
        prim_obj.insert("mode".to_owned(), json!(4));

        if let Some(mat_idx) = prim
            .material
            .or_else(|| (!gltf_materials.is_empty()).then_some(0))
        {
            prim_obj.insert("material".to_owned(), json!(mat_idx));
        }

        primitives_json.push(Value::Object(prim_obj));
    }

    let mut root = serde_json::Map::new();
    root.insert(
        "asset".to_owned(),
        json!({
            "version": "2.0",
            "generator": "scarlet-nexus-assets"
        }),
    );

    if !bin.bytes.is_empty() {
        root.insert(
            "buffers".to_owned(),
            json!([{
                "byteLength": bin.bytes.len()
            }]),
        );
    }
    if !buffer_views.is_empty() {
        root.insert("bufferViews".to_owned(), Value::Array(buffer_views));
    }
    if !accessors.is_empty() {
        root.insert("accessors".to_owned(), Value::Array(accessors));
    }
    if !primitives_json.is_empty() {
        let mesh_name = if asset.name.is_empty() {
            "mesh_0".to_owned()
        } else {
            asset.name.clone()
        };
        root.insert(
            "meshes".to_owned(),
            json!([{
                "name": mesh_name,
                "primitives": primitives_json
            }]),
        );
        root.insert(
            "nodes".to_owned(),
            json!([{
                "name": mesh_name,
                "mesh": 0
            }]),
        );
        root.insert(
            "scenes".to_owned(),
            json!([{
                "nodes": [0]
            }]),
        );
        root.insert("scene".to_owned(), json!(0));
    }
    if !gltf_materials.is_empty() {
        root.insert("materials".to_owned(), Value::Array(gltf_materials));
    }
    if !gltf_textures.is_empty() {
        root.insert("textures".to_owned(), Value::Array(gltf_textures));
    }
    if !images.is_empty() {
        root.insert("images".to_owned(), Value::Array(images));
    }

    let json_bytes =
        serde_json::to_vec(&Value::Object(root)).map_err(|e| SnError::InvalidData {
            resource: "glTF JSON",
            reason: e.to_string(),
        })?;

    let binary_opt = if bin.bytes.is_empty() {
        None
    } else {
        Some(bin.bytes.as_slice())
    };
    let glb_bytes = write_glb_container(
        &mut std::io::sink(),
        &json_bytes,
        binary_opt,
        limits.max_asset_bytes,
    )?;

    let report = ConversionReport {
        glb_bytes,
        primitive_count: asset.mesh.primitives.len(),
        vertex_count: usize::try_from(total_vertices).map_err(|_| SnError::SizeOverflow {
            resource: "total vertices",
        })?,
        index_count: usize::try_from(total_indices).map_err(|_| SnError::SizeOverflow {
            resource: "total indices",
        })?,
        texture_count: asset.textures.len(),
        texture_bytes: usize::try_from(total_texture_bytes).map_err(|_| SnError::SizeOverflow {
            resource: "total texture bytes",
        })?,
    };

    Ok(GlbDocument {
        json: json_bytes,
        binary: bin.bytes,
        report,
    })
}

pub fn write_prepared_glb<W: Write + ?Sized>(
    writer: &mut W,
    asset: &PreparedAsset,
    limits: &Limits,
) -> Result<ConversionReport> {
    let doc = build_glb_document(asset, limits)?;
    let binary_opt = if doc.binary.is_empty() {
        None
    } else {
        Some(doc.binary.as_slice())
    };
    let glb_bytes = write_glb_container(writer, &doc.json, binary_opt, limits.max_asset_bytes)?;
    debug_assert_eq!(glb_bytes, doc.report.glb_bytes);
    Ok(doc.report)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::glb::validate_glb;
    use crate::model::{MeshLod, Primitive, VertexAttributes};

    fn sample_prepared_asset() -> PreparedAsset {
        let png_1x1: Vec<u8> = vec![
            0x89, 0x50, 0x4E, 0x47, 0x0D, 0x0A, 0x1A, 0x0A, 0x00, 0x00, 0x00, 0x0D, 0x49, 0x48,
            0x44, 0x52, 0x00, 0x00, 0x00, 0x01, 0x00, 0x00, 0x00, 0x01, 0x08, 0x02, 0x00, 0x00,
            0x00, 0x90, 0x77, 0x53, 0xDE, 0x00, 0x00, 0x00, 0x0C, 0x49, 0x44, 0x41, 0x54, 0x08,
            0xD7, 0x63, 0xF8, 0xCF, 0xC0, 0x00, 0x00, 0x03, 0x01, 0x01, 0x00, 0x18, 0xDD, 0x8D,
            0xB0, 0x00, 0x00, 0x00, 0x00, 0x49, 0x45, 0x4E, 0x44, 0xAE, 0x42, 0x60, 0x82,
        ];
        PreparedAsset {
            name: "TriangleAsset".to_owned(),
            mesh: MeshLod {
                primitives: vec![Primitive {
                    attributes: VertexAttributes {
                        positions: vec![[0.0, 0.0, 0.0], [1.0, 0.0, 0.0], [0.0, 1.0, 0.0]],
                        normals: Some(vec![[0.0, 0.0, 1.0], [0.0, 0.0, 1.0], [0.0, 0.0, 1.0]]),
                        tangents: None,
                        texcoords: Some(vec![[0.0, 0.0], [1.0, 0.0], [0.0, 1.0]]),
                        joints: None,
                        weights: None,
                    },
                    indices: vec![0, 1, 2],
                    material: Some(0),
                }],
            },
            textures: vec![PreparedTexture {
                name: "AlbedoTex".to_owned(),
                width: 1,
                height: 1,
                color_space: ColorSpace::Srgb,
                png_bytes: png_1x1,
            }],
            materials: vec![Material {
                name: "TriangleMat".to_owned(),
                base_color_texture: Some(0),
                ..Material::default()
            }],
        }
    }

    #[test]
    fn bin_builder_writes_attributes_little_endian_with_alignment() {
        let mut bin = BinBuilder::new();
        bin.bytes.push(0xaa);

        let (uv_offset, uv_len) = bin
            .push_f32_2(&[[1.0, -2.0]])
            .expect("vec2 must be written");
        assert_eq!((uv_offset, uv_len), (4, 8));
        assert_eq!(&bin.bytes[..uv_offset], &[0xaa, 0, 0, 0]);
        assert_eq!(
            &bin.bytes[uv_offset..uv_offset + uv_len],
            [1.0_f32.to_le_bytes(), (-2.0_f32).to_le_bytes()]
                .concat()
                .as_slice()
        );

        let (vec3_offset, vec3_len) = bin
            .push_f32_3(&[[-0.0, 3.5, 4.0]])
            .expect("vec3 must be written");
        assert_eq!((vec3_offset, vec3_len), (12, 12));
        assert_eq!(
            &bin.bytes[vec3_offset..vec3_offset + vec3_len],
            [
                (-0.0_f32).to_le_bytes(),
                3.5_f32.to_le_bytes(),
                4.0_f32.to_le_bytes(),
            ]
            .concat()
            .as_slice()
        );

        let (vec4_offset, vec4_len) = bin
            .push_f32_4(&[[1.0, 2.0, 3.0, -1.0]])
            .expect("vec4 must be written");
        assert_eq!((vec4_offset, vec4_len), (24, 16));
        assert_eq!(
            &bin.bytes[vec4_offset..vec4_offset + vec4_len],
            [
                1.0_f32.to_le_bytes(),
                2.0_f32.to_le_bytes(),
                3.0_f32.to_le_bytes(),
                (-1.0_f32).to_le_bytes(),
            ]
            .concat()
            .as_slice()
        );

        let (joints_offset, joints_len) = bin
            .push_u16_4(&[[0, 255, 256, u16::MAX]])
            .expect("joints must be written");
        assert_eq!((joints_offset, joints_len), (40, 8));
        assert_eq!(
            &bin.bytes[joints_offset..joints_offset + joints_len],
            [
                0_u16.to_le_bytes(),
                255_u16.to_le_bytes(),
                256_u16.to_le_bytes(),
                u16::MAX.to_le_bytes(),
            ]
            .concat()
            .as_slice()
        );
    }

    #[test]
    fn bin_builder_writes_each_index_width_little_endian() {
        let mut bin = BinBuilder::new();

        let (u8_offset, u8_len, u8_type) = bin
            .push_indices(&[0, u32::from(u8::MAX), 2])
            .expect("u8 indices must be written");
        assert_eq!((u8_offset, u8_len, u8_type), (0, 3, 5121));
        assert_eq!(bin.bytes, [0, u8::MAX, 2, 0]);

        let (u16_offset, u16_len, u16_type) = bin
            .push_indices(&[256, u32::from(u16::MAX)])
            .expect("u16 indices must be written");
        assert_eq!((u16_offset, u16_len, u16_type), (4, 4, 5123));
        assert_eq!(
            &bin.bytes[u16_offset..u16_offset + u16_len],
            &[0, 1, 255, 255]
        );

        let (u32_offset, u32_len, u32_type) = bin
            .push_indices(&[65_536, 0x0102_0304])
            .expect("u32 indices must be written");
        assert_eq!((u32_offset, u32_len, u32_type), (8, 8, 5125));
        assert_eq!(
            &bin.bytes[u32_offset..u32_offset + u32_len],
            &[0, 0, 1, 0, 4, 3, 2, 1]
        );
    }

    #[test]
    fn builds_and_validates_glb_with_mesh_and_texture() {
        let asset = sample_prepared_asset();
        let limits = Limits::default();
        let mut out = Vec::new();
        let report = write_prepared_glb(&mut out, &asset, &limits).expect("GLB must write");
        assert_eq!(report.vertex_count, 3);
        assert_eq!(report.index_count, 3);
        assert_eq!(report.texture_count, 1);
        assert_eq!(report.glb_bytes, out.len());
        validate_glb(&out, limits.max_asset_bytes).expect("GLB container must be valid");

        let parsed = gltf::Glb::from_slice(&out).expect("crate gltf must parse generated GLB");
        assert!(parsed.bin.is_some());
        let json_val: Value =
            serde_json::from_slice(&parsed.json).expect("embedded glTF JSON must parse");
        assert_eq!(json_val["asset"]["version"], "2.0");
        assert_eq!(json_val["meshes"][0]["primitives"][0]["indices"], 3);
        assert_eq!(
            json_val["materials"][0]["pbrMetallicRoughness"]["baseColorTexture"]["index"],
            0
        );
    }

    #[test]
    fn fallback_material_does_not_assign_a_texture() {
        let mut asset = sample_prepared_asset();
        asset.materials.clear();
        for primitive in &mut asset.mesh.primitives {
            primitive.material = None;
        }
        let document = build_glb_document(&asset, &Limits::default())
            .expect("asset without materials must use a neutral fallback");
        let json: Value = serde_json::from_slice(&document.json).expect("valid glTF JSON");
        let pbr = json["materials"][0]["pbrMetallicRoughness"]
            .as_object()
            .expect("PBR object");
        assert!(!pbr.contains_key("baseColorTexture"));
        assert_eq!(json["images"][0]["bufferView"], 0);
    }

    #[test]
    fn rejects_aggregate_texture_bytes_before_building_buffers() {
        let mut asset = sample_prepared_asset();
        asset.textures.push(asset.textures[0].clone());
        let texture_bytes =
            u64::try_from(asset.textures[0].png_bytes.len()).expect("texture length fits u64");
        let limits = Limits {
            max_texture_bytes: texture_bytes,
            ..Limits::default()
        };
        let error = build_glb_document(&asset, &limits)
            .expect_err("both textures must exceed the aggregate byte budget");
        assert!(matches!(
            error,
            SnError::LimitExceeded {
                resource: "texture bytes",
                actual,
                limit
            } if actual == texture_bytes * 2 && limit == texture_bytes
        ));
    }

    #[test]
    fn rejects_oversized_binary_before_building_buffers() {
        let asset = sample_prepared_asset();
        let limits = Limits {
            max_asset_bytes: 171,
            ..Limits::default()
        };
        let error = build_glb_document(&asset, &limits)
            .expect_err("172-byte binary payload must exceed its budget before allocation");
        assert!(
            matches!(
                error,
                SnError::LimitExceeded {
                    resource: "GLB binary buffer",
                    actual: 172,
                    limit: 171
                }
            ),
            "unexpected error: {error:?}"
        );
    }

    #[test]
    fn rejects_exceeding_aggregate_vertex_limit() {
        let asset = sample_prepared_asset();
        let limits = Limits {
            max_vertices: 2,
            ..Limits::default()
        };
        let err = build_glb_document(&asset, &limits).expect_err("must exceed vertex limit");
        assert!(matches!(
            err,
            SnError::LimitExceeded {
                resource: "vertices",
                actual: 3,
                limit: 2
            }
        ));
    }
}
