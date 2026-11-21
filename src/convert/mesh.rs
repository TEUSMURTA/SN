use crate::{
    config::ConversionOptions,
    error::{Result, SnError},
    model::{MeshAsset, MeshLod, Primitive, VertexAttributes},
};

/// Converts one LOD from Unreal's X-forward/Y-right/Z-up coordinates to glTF's
/// X-right/Y-up/Z-forward coordinates. The reflection also reverses triangle winding.
pub fn convert_mesh(mesh: &MeshAsset, options: &ConversionOptions) -> Result<MeshLod> {
    if mesh.skeleton.is_some() {
        return Err(SnError::Unsupported(
            "skeletal mesh export is not implemented".to_owned(),
        ));
    }

    let source = mesh
        .lods
        .get(options.lod)
        .ok_or_else(|| SnError::InvalidData {
            resource: "mesh LOD",
            reason: format!(
                "LOD {} was requested, but only {} LODs exist",
                options.lod,
                mesh.lods.len()
            ),
        })?;
    if source.primitives.is_empty() {
        return Err(SnError::InvalidData {
            resource: "mesh",
            reason: "at least one primitive is required".to_owned(),
        });
    }

    source
        .primitives
        .iter()
        .map(|primitive| convert_primitive(primitive, options))
        .collect::<Result<Vec<_>>>()
        .map(|primitives| MeshLod { primitives })
}

fn convert_primitive(primitive: &Primitive, options: &ConversionOptions) -> Result<Primitive> {
    let source = &primitive.attributes;
    let vertex_count = source.positions.len();
    if vertex_count == 0 {
        return Err(SnError::InvalidData {
            resource: "vertices",
            reason: "POSITION cannot be empty".to_owned(),
        });
    }
    check_limit(
        "vertices",
        vertex_count,
        options.limits.max_vertices,
    )?;
    check_limit("indices", primitive.indices.len(), options.limits.max_indices)?;
    if primitive.indices.is_empty() || primitive.indices.len() % 3 != 0 {
        return Err(SnError::InvalidData {
            resource: "indices",
            reason: "indices must contain complete, non-empty triangles".to_owned(),
        });
    }

    check_attribute_len("normals", source.normals.as_ref().map(Vec::len), vertex_count)?;
    check_attribute_len("tangents", source.tangents.as_ref().map(Vec::len), vertex_count)?;
    check_attribute_len(
        "texture coordinates",
        source.texcoords.as_ref().map(Vec::len),
        vertex_count,
    )?;
    check_attribute_len("joints", source.joints.as_ref().map(Vec::len), vertex_count)?;
    check_attribute_len("weights", source.weights.as_ref().map(Vec::len), vertex_count)?;
    if source.joints.is_some() || source.weights.is_some() {
        return Err(SnError::Unsupported(
            "JOINTS_0 and WEIGHTS_0 export is not implemented".to_owned(),
        ));
    }

    let positions = source
        .positions
        .iter()
        .map(|position| convert_position(*position))
        .collect::<Result<Vec<_>>>()?;
    let normals = source
        .normals
        .as_ref()
        .map(|values| {
            values
                .iter()
                .map(|normal| convert_normal(*normal, "normal"))
                .collect::<Result<Vec<_>>>()
        })
        .transpose()?;
    let tangents = source
        .tangents
        .as_ref()
        .map(|values| {
            values
                .iter()
                .map(|tangent| convert_tangent(*tangent))
                .collect::<Result<Vec<_>>>()
        })
        .transpose()?;
    let texcoords = source
        .texcoords
        .as_ref()
        .map(|values| {
            values
                .iter()
                .map(|uv| convert_uv(*uv, options.flip_v))
                .collect::<Result<Vec<_>>>()
        })
        .transpose()?;

    let mut indices = Vec::with_capacity(primitive.indices.len());
    for triangle in primitive.indices.chunks_exact(3) {
        for index in triangle {
            let index = usize::try_from(*index).map_err(|_| SnError::InvalidData {
                resource: "indices",
                reason: format!("index {index} does not fit in usize"),
            })?;
            if index >= vertex_count {
                return Err(SnError::InvalidData {
                    resource: "indices",
                    reason: format!("index {index} is outside a vertex count of {vertex_count}"),
                });
            }
        }
        indices.extend_from_slice(&[triangle[0], triangle[2], triangle[1]]);
    }

    Ok(Primitive {
        attributes: VertexAttributes {
            positions,
            normals,
            tangents,
            texcoords,
            joints: None,
            weights: None,
        },
        indices,
        material: primitive.material,
    })
}

fn check_limit(resource: &'static str, actual: usize, limit: usize) -> Result<()> {
    if actual > limit {
        return Err(SnError::LimitExceeded {
            resource,
            actual: actual as u64,
            limit: limit as u64,
        });
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

fn convert_position(position: [f32; 3]) -> Result<[f32; 3]> {
    ensure_finite(&position, "positions")?;
    Ok([-position[1], position[2], position[0]])
}

fn convert_normal(normal: [f32; 3], resource: &'static str) -> Result<[f32; 3]> {
    ensure_finite(&normal, resource)?;
    let converted = [-normal[1], normal[2], normal[0]];
    normalize(converted, resource)
}

fn convert_tangent(tangent: [f32; 4]) -> Result<[f32; 4]> {
    ensure_finite(&tangent, "tangents")?;
    let xyz = normalize(
        [-tangent[1], tangent[2], tangent[0]],
        "tangents",
    )?;
    if tangent[3] == 0.0 {
        return Err(SnError::InvalidData {
            resource: "tangents",
            reason: "tangent handedness cannot be zero".to_owned(),
        });
    }
    // A reflection reverses the cross product used to derive the bitangent.
    Ok([xyz[0], xyz[1], xyz[2], -tangent[3].signum()])
}

fn convert_uv(uv: [f32; 2], flip_v: bool) -> Result<[f32; 2]> {
    ensure_finite(&uv, "texture coordinates")?;
    Ok([
        uv[0],
        if flip_v { 1.0 - uv[1] } else { uv[1] },
    ])
}

fn ensure_finite(values: &[f32], resource: &'static str) -> Result<()> {
    if values.iter().any(|value| !value.is_finite()) {
        return Err(SnError::InvalidData {
            resource,
            reason: "all floating-point values must be finite".to_owned(),
        });
    }
    Ok(())
}

fn normalize(vector: [f32; 3], resource: &'static str) -> Result<[f32; 3]> {
    let length_squared = vector.iter().map(|value| value * value).sum::<f32>();
    if !length_squared.is_finite() || length_squared <= f32::EPSILON {
        return Err(SnError::InvalidData {
            resource,
            reason: "direction must have a finite, non-zero length".to_owned(),
        });
    }
    let inverse_length = length_squared.sqrt().recip();
    Ok([
        vector[0] * inverse_length,
        vector[1] * inverse_length,
        vector[2] * inverse_length,
    ])
}

#[cfg(test)]
mod tests {
    use super::*;

    fn mesh_with(primitive: Primitive) -> MeshAsset {
        MeshAsset {
            name: "test".to_owned(),
            lods: vec![MeshLod {
                primitives: vec![primitive],
            }],
            skeleton: None,
        }
    }

    fn triangle() -> Primitive {
        Primitive {
            attributes: VertexAttributes {
                positions: vec![[0.0, 0.0, 0.0], [1.0, 0.0, 0.0], [0.0, 1.0, 0.0]],
                normals: Some(vec![[0.0, 0.0, 2.0]; 3]),
                tangents: Some(vec![[1.0, 0.0, 0.0, 1.0]; 3]),
                texcoords: Some(vec![[0.0, 0.0], [1.0, 0.0], [0.0, 1.0]]),
                joints: None,
                weights: None,
            },
            indices: vec![0, 1, 2],
            material: None,
        }
    }

    #[test]
    fn converts_axes_normals_tangents_and_winding() {
        let converted = convert_mesh(&mesh_with(triangle()), &ConversionOptions::default())
            .expect("triangle must convert");
        let attributes = &converted.primitives[0].attributes;
        assert_eq!(
            attributes.positions,
            [[0.0, 0.0, 0.0], [0.0, 0.0, 1.0], [-1.0, 0.0, 0.0]]
        );
        assert_eq!(attributes.normals.as_ref().expect("normals")[0], [0.0, 1.0, 0.0]);
        assert_eq!(
            attributes.tangents.as_ref().expect("tangents")[0],
            [0.0, 0.0, 1.0, -1.0]
        );
        assert_eq!(converted.primitives[0].indices, [0, 2, 1]);
    }

    #[test]
    fn rejects_out_of_bounds_indices() {
        let mut primitive = triangle();
        primitive.indices[2] = 3;
        let error = convert_mesh(&mesh_with(primitive), &ConversionOptions::default())
            .expect_err("index must be rejected");
        assert!(matches!(
            error,
            SnError::InvalidData {
                resource: "indices",
                ..
            }
        ));
    }

    #[test]
    fn rejects_attribute_count_mismatch() {
        let mut primitive = triangle();
        primitive.attributes.normals = Some(vec![[0.0, 0.0, 1.0]; 2]);
        let error = convert_mesh(&mesh_with(primitive), &ConversionOptions::default())
            .expect_err("normal count must match positions");
        assert!(matches!(
            error,
            SnError::InvalidData {
                resource: "normals",
                ..
            }
        ));
    }

    #[test]
    fn rejects_non_finite_positions() {
        let mut primitive = triangle();
        primitive.attributes.positions[0][0] = f32::NAN;
        let error = convert_mesh(&mesh_with(primitive), &ConversionOptions::default())
            .expect_err("NaN must be rejected");
        assert!(matches!(
            error,
            SnError::InvalidData {
                resource: "positions",
                ..
            }
        ));
    }

    #[test]
    fn reports_missing_lod() {
        let options = ConversionOptions {
            lod: 1,
            ..ConversionOptions::default()
        };
        let error = convert_mesh(&mesh_with(triangle()), &options)
            .expect_err("LOD 1 must not exist");
        assert!(matches!(
            error,
            SnError::InvalidData {
                resource: "mesh LOD",
                ..
            }
        ));
    }
}
