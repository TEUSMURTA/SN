use std::{
    collections::{BTreeMap, HashMap},
    path::Path,
};

use crate::{
    config::Limits,
    error::{Result, SnError},
    io::reader::{AssetReader, checked_byte_len, read_file_bounded},
    model::{MeshAsset, MeshLod, Primitive, VertexAttributes},
};

#[cfg(test)]
const CHUNK_HEADER_LEN: usize = 32;
const MAX_MATERIALS: usize = 256;

/// Reads a static ActorX PSK/PSKX mesh exported by UModel.
pub fn read_umodel_mesh(path: impl AsRef<Path>, limits: &Limits) -> Result<MeshAsset> {
    let path = path.as_ref();
    if !path.extension().is_some_and(|extension| {
        extension.eq_ignore_ascii_case("psk") || extension.eq_ignore_ascii_case("pskx")
    }) {
        return Err(SnError::Unsupported(
            "only PSK and PSKX mesh exports are currently supported".to_owned(),
        ));
    }
    let bytes = read_file_bounded(
        path,
        limits.max_input_bytes,
        "reading UModel mesh",
        "PSK mesh",
    )?;
    let name = path
        .file_stem()
        .map(|stem| stem.to_string_lossy().into_owned())
        .unwrap_or_else(|| "mesh".to_owned());
    parse_psk(&name, &bytes, limits)
}

/// Parses a static ActorX mesh. Wedges become vertices so UV seams survive export.
pub fn parse_psk(name: &str, input: &[u8], limits: &Limits) -> Result<MeshAsset> {
    let input_len = u64::try_from(input.len()).map_err(|_| SnError::SizeOverflow {
        resource: "PSK mesh",
    })?;
    if input_len > limits.max_input_bytes {
        return Err(SnError::LimitExceeded {
            resource: "PSK mesh",
            actual: input_len,
            limit: limits.max_input_bytes,
        });
    }

    let mut reader = AssetReader::new(input, limits.max_input_bytes);
    let mut chunks = Chunks::default();
    let mut saw_header = false;
    while reader.offset() < input_len {
        let id_bytes = reader.read_array::<20>()?;
        let id_len = id_bytes
            .iter()
            .position(|byte| *byte == 0)
            .unwrap_or(id_bytes.len());
        let id = String::from_utf8_lossy(&id_bytes[..id_len]);
        let _type_flags = reader.read_u32_le()?;
        let element_size = u64::from(reader.read_u32_le()?);
        let count = u64::from(reader.read_u32_le()?);
        let payload_len = chunk_payload_len(count, element_size, input_len)?;

        match id.as_ref() {
            "ACTRHEAD" => {
                reject_duplicate(&mut saw_header, "PSK header")?;
                skip_payload(&mut reader, payload_len)?;
            }
            "PNTS0000" => {
                reject_duplicate(&mut chunks.points.is_some(), "PSK points")?;
                check_record_size(element_size, 12, "PSK points")?;
                check_count(count, limits.max_vertices, "PSK points")?;
                chunks.points = Some(read_points(&mut reader, count)?);
            }
            "VTXW0000" | "VTXW3200" => {
                reject_duplicate(&mut chunks.wedges.is_some(), "PSK wedges")?;
                check_record_size(element_size, 16, "PSK wedges")?;
                check_count(count, limits.max_indices, "PSK wedges")?;
                chunks.wedges = Some(read_wedges(&mut reader, count, id == "VTXW3200")?);
            }
            "FACE0000" => {
                reject_duplicate(&mut chunks.faces.is_some(), "PSK faces")?;
                check_record_size(element_size, 12, "PSK faces")?;
                let corners = count.checked_mul(3).ok_or(SnError::SizeOverflow {
                    resource: "PSK indices",
                })?;
                check_count(corners, limits.max_indices, "PSK indices")?;
                chunks.faces = Some(read_faces(&mut reader, count)?);
            }
            "MATT0000" => {
                reject_duplicate(&mut chunks.material_count.is_some(), "PSK materials")?;
                check_record_size(element_size, 88, "PSK materials")?;
                check_count(count, MAX_MATERIALS, "PSK materials")?;
                chunks.material_count =
                    Some(usize::try_from(count).map_err(|_| SnError::SizeOverflow {
                        resource: "PSK materials",
                    })?);
                skip_payload(&mut reader, payload_len)?;
            }
            "REFSKELT" | "RAWWEIGHTS" => {
                return Err(SnError::Unsupported(
                    "skeletal PSK meshes are not implemented".to_owned(),
                ));
            }
            _ => skip_payload(&mut reader, payload_len)?,
        }
    }

    if !saw_header {
        return Err(SnError::InvalidData {
            resource: "PSK header",
            reason: "ACTRHEAD chunk is missing".to_owned(),
        });
    }
    build_mesh(name, chunks, limits)
}

#[derive(Default)]
struct Chunks {
    points: Option<Vec<[f32; 3]>>,
    wedges: Option<Vec<Wedge>>,
    faces: Option<Vec<Face>>,
    material_count: Option<usize>,
}

#[derive(Clone, Copy)]
struct Wedge {
    point_index: u32,
    uv: [f32; 2],
}

#[derive(Clone, Copy)]
struct Face {
    wedges: [u16; 3],
    material: u8,
}

fn chunk_payload_len(count: u64, element_size: u64, max_bytes: u64) -> Result<usize> {
    checked_byte_len(count, element_size, max_bytes, "PSK chunk payload")
}

fn check_record_size(actual: u64, expected: u64, resource: &'static str) -> Result<()> {
    if actual != expected {
        return Err(SnError::InvalidData {
            resource,
            reason: format!("chunk record size is {actual} bytes, expected {expected}"),
        });
    }
    Ok(())
}

fn check_count(actual: u64, limit: usize, resource: &'static str) -> Result<()> {
    let limit = u64::try_from(limit).map_err(|_| SnError::SizeOverflow { resource })?;
    if actual > limit {
        return Err(SnError::LimitExceeded {
            resource,
            actual,
            limit,
        });
    }
    Ok(())
}

fn reject_duplicate(seen: &mut bool, resource: &'static str) -> Result<()> {
    if *seen {
        return Err(SnError::InvalidData {
            resource,
            reason: "chunk appears more than once".to_owned(),
        });
    }
    *seen = true;
    Ok(())
}

fn skip_payload(reader: &mut AssetReader<&[u8]>, payload_len: usize) -> Result<()> {
    reader.read_bytes(payload_len, "PSK chunk payload")?;
    Ok(())
}

fn read_points(reader: &mut AssetReader<&[u8]>, count: u64) -> Result<Vec<[f32; 3]>> {
    let count = to_usize(count, "PSK points")?;
    let mut points = Vec::new();
    points
        .try_reserve_exact(count)
        .map_err(|_| SnError::AllocationFailed {
            resource: "PSK points",
        })?;
    for _ in 0..count {
        points.push([
            reader.read_f32_le()?,
            reader.read_f32_le()?,
            reader.read_f32_le()?,
        ]);
    }
    Ok(points)
}

fn read_wedges(
    reader: &mut AssetReader<&[u8]>,
    count: u64,
    wide_indices: bool,
) -> Result<Vec<Wedge>> {
    let count = to_usize(count, "PSK wedges")?;
    let mut wedges = Vec::new();
    wedges
        .try_reserve_exact(count)
        .map_err(|_| SnError::AllocationFailed {
            resource: "PSK wedges",
        })?;
    for _ in 0..count {
        let point_index = if wide_indices {
            reader.read_u32_le()?
        } else {
            u32::from(reader.read_u16_le()?)
        };
        if !wide_indices {
            let _padding = reader.read_u16_le()?;
        }
        let uv = [reader.read_f32_le()?, reader.read_f32_le()?];
        let _material = reader.read_u8()?;
        let _reserved = reader.read_u8()?;
        let _padding = reader.read_u16_le()?;
        wedges.push(Wedge { point_index, uv });
    }
    Ok(wedges)
}

fn read_faces(reader: &mut AssetReader<&[u8]>, count: u64) -> Result<Vec<Face>> {
    let count = to_usize(count, "PSK faces")?;
    let mut faces = Vec::new();
    faces
        .try_reserve_exact(count)
        .map_err(|_| SnError::AllocationFailed {
            resource: "PSK faces",
        })?;
    for _ in 0..count {
        let wedges = [
            reader.read_u16_le()?,
            reader.read_u16_le()?,
            reader.read_u16_le()?,
        ];
        let material = reader.read_u8()?;
        let _aux_material = reader.read_u8()?;
        let _smoothing_groups = reader.read_u32_le()?;
        faces.push(Face { wedges, material });
    }
    Ok(faces)
}

fn to_usize(value: u64, resource: &'static str) -> Result<usize> {
    usize::try_from(value).map_err(|_| SnError::SizeOverflow { resource })
}

#[derive(Default)]
struct PrimitiveBuilder {
    positions: Vec<[f32; 3]>,
    texcoords: Vec<[f32; 2]>,
    indices: Vec<u32>,
    wedge_vertices: HashMap<u16, u32>,
}

fn build_mesh(name: &str, chunks: Chunks, limits: &Limits) -> Result<MeshAsset> {
    let points = required(chunks.points, "PSK points")?;
    let wedges = required(chunks.wedges, "PSK wedges")?;
    let faces = required(chunks.faces, "PSK faces")?;
    if points.is_empty() || wedges.is_empty() || faces.is_empty() {
        return Err(SnError::InvalidData {
            resource: "PSK mesh",
            reason: "points, wedges and faces must not be empty".to_owned(),
        });
    }
    if let Some(material_count) = chunks.material_count {
        for face in &faces {
            if usize::from(face.material) >= material_count {
                return Err(SnError::InvalidData {
                    resource: "PSK materials",
                    reason: format!(
                        "face references material {}, but only {material_count} materials exist",
                        face.material
                    ),
                });
            }
        }
    }

    let mut builders: BTreeMap<u8, PrimitiveBuilder> = BTreeMap::new();
    let mut total_vertices = 0_u64;
    for face in &faces {
        let builder = builders.entry(face.material).or_default();
        builder
            .indices
            .try_reserve(3)
            .map_err(|_| SnError::AllocationFailed {
                resource: "PSK indices",
            })?;
        for wedge_index in face.wedges {
            let output_index = if let Some(index) = builder.wedge_vertices.get(&wedge_index) {
                *index
            } else {
                let wedge =
                    wedges
                        .get(usize::from(wedge_index))
                        .ok_or_else(|| SnError::InvalidData {
                            resource: "PSK faces",
                            reason: format!("wedge index {wedge_index} is out of bounds"),
                        })?;
                let point = points
                    .get(
                        usize::try_from(wedge.point_index).map_err(|_| SnError::SizeOverflow {
                            resource: "PSK point index",
                        })?,
                    )
                    .ok_or_else(|| SnError::InvalidData {
                        resource: "PSK wedges",
                        reason: format!("point index {} is out of bounds", wedge.point_index),
                    })?;
                total_vertices = total_vertices.checked_add(1).ok_or(SnError::SizeOverflow {
                    resource: "PSK output vertices",
                })?;
                check_count(total_vertices, limits.max_vertices, "vertices")?;
                let output_index =
                    u32::try_from(builder.positions.len()).map_err(|_| SnError::SizeOverflow {
                        resource: "PSK vertex index",
                    })?;
                builder
                    .positions
                    .try_reserve(1)
                    .map_err(|_| SnError::AllocationFailed {
                        resource: "PSK output vertices",
                    })?;
                builder
                    .texcoords
                    .try_reserve(1)
                    .map_err(|_| SnError::AllocationFailed {
                        resource: "PSK output texture coordinates",
                    })?;
                builder
                    .wedge_vertices
                    .try_reserve(1)
                    .map_err(|_| SnError::AllocationFailed {
                        resource: "PSK wedge map",
                    })?;
                builder.positions.push(*point);
                builder.texcoords.push(wedge.uv);
                builder.wedge_vertices.insert(wedge_index, output_index);
                output_index
            };
            builder.indices.push(output_index);
        }
    }

    let primitives = builders
        .into_iter()
        .map(|(material, builder)| Primitive {
            attributes: VertexAttributes {
                positions: builder.positions,
                normals: None,
                tangents: None,
                texcoords: Some(builder.texcoords),
                joints: None,
                weights: None,
            },
            indices: builder.indices,
            material: chunks.material_count.map(|_| usize::from(material)),
        })
        .collect();
    Ok(MeshAsset {
        name: name.to_owned(),
        lods: vec![MeshLod { primitives }],
        skeleton: None,
    })
}

fn required<T>(value: Option<T>, resource: &'static str) -> Result<T> {
    value.ok_or_else(|| SnError::InvalidData {
        resource,
        reason: "required chunk is missing".to_owned(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn chunk(id: &[u8], element_size: u32, _values: &[u8]) -> Vec<u8> {
        assert!(id.len() <= 20);
        assert_eq!(element_size, 0);
        let mut bytes = vec![0_u8; CHUNK_HEADER_LEN];
        bytes[..id.len()].copy_from_slice(id);
        bytes
    }

    fn records(id: &[u8], element_size: u32, count: u32, payload: &[u8]) -> Vec<u8> {
        assert!(id.len() <= 20);
        assert_eq!(payload.len(), element_size as usize * count as usize);
        let mut bytes = vec![0_u8; CHUNK_HEADER_LEN];
        bytes[..id.len()].copy_from_slice(id);
        bytes[24..28].copy_from_slice(&element_size.to_le_bytes());
        bytes[28..32].copy_from_slice(&count.to_le_bytes());
        bytes.extend_from_slice(payload);
        bytes
    }

    fn triangle() -> Vec<u8> {
        let mut bytes = chunk(b"ACTRHEAD", 0, &[]);
        let mut points = Vec::new();
        for point in [[0.0_f32, 0.0, 0.0], [1.0, 0.0, 0.0], [0.0, 1.0, 0.0]] {
            for component in point {
                points.extend_from_slice(&component.to_le_bytes());
            }
        }
        bytes.extend_from_slice(&records(b"PNTS0000", 12, 3, &points));
        let mut wedges = Vec::new();
        for (point, uv) in [
            (0_u16, [0.0_f32, 0.0]),
            (1_u16, [1.0, 0.0]),
            (2_u16, [0.0, 1.0]),
        ] {
            wedges.extend_from_slice(&point.to_le_bytes());
            wedges.extend_from_slice(&0_u16.to_le_bytes());
            wedges.extend_from_slice(&uv[0].to_le_bytes());
            wedges.extend_from_slice(&uv[1].to_le_bytes());
            wedges.extend_from_slice(&[0, 0]);
            wedges.extend_from_slice(&0_u16.to_le_bytes());
        }
        bytes.extend_from_slice(&records(b"VTXW0000", 16, 3, &wedges));
        let mut face = Vec::new();
        for wedge in [0_u16, 1, 2] {
            face.extend_from_slice(&wedge.to_le_bytes());
        }
        face.extend_from_slice(&[0, 0]);
        face.extend_from_slice(&1_u32.to_le_bytes());
        bytes.extend_from_slice(&records(b"FACE0000", 12, 1, &face));
        bytes.extend_from_slice(&records(b"MATT0000", 88, 1, &[0; 88]));
        bytes
    }

    #[test]
    fn parses_triangle_and_rebuilds_wedge_vertices() {
        let mesh =
            parse_psk("triangle", &triangle(), &Limits::default()).expect("valid PSK must parse");
        let primitive = &mesh.lods[0].primitives[0];
        assert_eq!(mesh.name, "triangle");
        assert_eq!(primitive.attributes.positions.len(), 3);
        assert_eq!(
            primitive.attributes.texcoords.as_ref().expect("UVs")[1],
            [1.0, 0.0]
        );
        assert_eq!(primitive.indices, [0, 1, 2]);
        assert_eq!(primitive.material, Some(0));
    }

    #[test]
    fn enforces_aggregate_vertex_limit_across_materials() {
        let chunks = Chunks {
            points: Some(vec![[0.0, 0.0, 0.0], [1.0, 0.0, 0.0], [0.0, 1.0, 0.0]]),
            wedges: Some(vec![
                Wedge {
                    point_index: 0,
                    uv: [0.0, 0.0],
                },
                Wedge {
                    point_index: 1,
                    uv: [1.0, 0.0],
                },
                Wedge {
                    point_index: 2,
                    uv: [0.0, 1.0],
                },
            ]),
            faces: Some(vec![
                Face {
                    wedges: [0, 1, 2],
                    material: 0,
                },
                Face {
                    wedges: [0, 1, 2],
                    material: 1,
                },
            ]),
            material_count: Some(2),
        };
        let limits = Limits {
            max_vertices: 5,
            ..Limits::default()
        };
        let error = build_mesh("two-materials", chunks, &limits)
            .expect_err("vertices must be limited across all primitives");
        assert!(matches!(
            error,
            SnError::LimitExceeded {
                resource: "vertices",
                actual: 6,
                limit: 5
            }
        ));
    }

    #[test]
    fn reports_truncated_chunk_payload_as_eof() {
        let mut bytes = chunk(b"ACTRHEAD", 0, &[]);
        bytes.extend_from_slice(&records(b"PNTS0000", 12, 1, &[0; 12]));
        bytes.truncate(bytes.len() - 1);
        let error =
            parse_psk("short", &bytes, &Limits::default()).expect_err("truncated point must fail");
        assert!(matches!(
            error,
            SnError::UnexpectedEof {
                offset: 72,
                requested: 4
            }
        ));
    }

    #[test]
    fn rejects_count_over_limit_before_reading_payload() {
        let mut bytes = chunk(b"ACTRHEAD", 0, &[]);
        bytes.extend_from_slice(&records(b"PNTS0000", 12, 2, &[0; 24]));
        bytes.truncate(CHUNK_HEADER_LEN * 2);
        let limits = Limits {
            max_vertices: 1,
            ..Limits::default()
        };
        let error =
            parse_psk("large", &bytes, &limits).expect_err("point count must exceed its limit");
        assert!(matches!(
            error,
            SnError::LimitExceeded {
                resource: "PSK points",
                actual: 2,
                limit: 1
            }
        ));
    }

    #[test]
    fn rejects_face_count_that_exceeds_index_budget() {
        let limits = Limits {
            max_indices: 2,
            ..Limits::default()
        };
        let error = check_count(3, limits.max_indices, "PSK indices")
            .expect_err("one triangle needs three indices");
        assert!(matches!(
            error,
            SnError::LimitExceeded {
                actual: 3,
                limit: 2,
                ..
            }
        ));
    }

    #[test]
    fn detects_chunk_size_overflow() {
        let error = chunk_payload_len(u64::MAX, 2, u64::MAX)
            .expect_err("chunk multiplication must overflow");
        assert!(matches!(
            error,
            SnError::SizeOverflow {
                resource: "PSK chunk payload"
            }
        ));
    }

    #[test]
    fn rejects_out_of_bounds_point_index() {
        let mut bytes = triangle();
        // ACTRHEAD + PNTS header/payload + VTXW header = first wedge.
        let point_offset = CHUNK_HEADER_LEN * 3 + 36;
        bytes[point_offset..point_offset + 2].copy_from_slice(&3_u16.to_le_bytes());
        let error = parse_psk("bad-index", &bytes, &Limits::default())
            .expect_err("point index must be rejected");
        assert!(matches!(
            error,
            SnError::InvalidData {
                resource: "PSK wedges",
                ..
            }
        ));
    }

    #[test]
    fn rejects_input_beyond_asset_budget() {
        let limits = Limits {
            max_input_bytes: 4,
            ..Limits::default()
        };
        let error =
            parse_psk("large", &triangle(), &limits).expect_err("input must exceed byte budget");
        assert!(matches!(error, SnError::LimitExceeded { .. }));
    }
}
