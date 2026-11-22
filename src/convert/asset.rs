use crate::{
    config::{ConversionOptions, Limits},
    convert::{
        mesh::{convert_mesh, selected_lod},
        texture::{ensure_supported_target, prepare_validated_texture, validate_texture},
    },
    error::{Result, SnError},
    model::{Asset, MeshLod, PreparedAsset},
};

#[derive(Debug, Default)]
struct AssetUsage {
    vertices: u64,
    indices: u64,
    rgba_bytes: u64,
    encoded_texture_bytes: u64,
}

impl AssetUsage {
    fn add_mesh(&mut self, mesh: &MeshLod, limits: &Limits) -> Result<()> {
        let vertex_limit =
            u64::try_from(limits.max_vertices).map_err(|_| SnError::SizeOverflow {
                resource: "vertices limit",
            })?;
        let index_limit = u64::try_from(limits.max_indices).map_err(|_| SnError::SizeOverflow {
            resource: "indices limit",
        })?;

        for primitive in &mesh.primitives {
            let vertices = u64::try_from(primitive.attributes.positions.len()).map_err(|_| {
                SnError::SizeOverflow {
                    resource: "vertices",
                }
            })?;
            self.vertices = checked_add_usage(self.vertices, vertices, vertex_limit, "vertices")?;
            let indices =
                u64::try_from(primitive.indices.len()).map_err(|_| SnError::SizeOverflow {
                    resource: "indices",
                })?;
            self.indices = checked_add_usage(self.indices, indices, index_limit, "indices")?;
        }
        Ok(())
    }

    fn add_rgba(&mut self, bytes: u64, limits: &Limits) -> Result<()> {
        self.rgba_bytes = checked_add_usage(
            self.rgba_bytes,
            bytes,
            limits.max_asset_rgba_bytes,
            "asset RGBA bytes",
        )?;
        Ok(())
    }

    fn add_encoded_texture(&mut self, bytes: usize, limits: &Limits) -> Result<()> {
        let bytes = u64::try_from(bytes).map_err(|_| SnError::SizeOverflow {
            resource: "encoded texture bytes",
        })?;
        self.encoded_texture_bytes = checked_add_usage(
            self.encoded_texture_bytes,
            bytes,
            limits.max_asset_encoded_texture_bytes,
            "asset encoded texture bytes",
        )?;
        Ok(())
    }
}

fn checked_add_usage(total: u64, amount: u64, limit: u64, resource: &'static str) -> Result<u64> {
    let updated = total
        .checked_add(amount)
        .ok_or(SnError::SizeOverflow { resource })?;
    if updated > limit {
        return Err(SnError::LimitExceeded {
            resource,
            actual: updated,
            limit,
        });
    }
    Ok(updated)
}

/// Converts one complete asset while enforcing shared geometry and texture budgets.
pub fn prepare_asset(asset: &Asset, options: &ConversionOptions) -> Result<PreparedAsset> {
    options.validate()?;
    ensure_supported_target(options)?;

    let mut usage = AssetUsage::default();
    usage.add_mesh(selected_lod(&asset.mesh, options.lod)?, &options.limits)?;

    // Validate every source and reject the aggregate RGBA budget before encoding.
    for texture in &asset.textures {
        usage.add_rgba(validate_texture(texture, options)?, &options.limits)?;
    }

    let mesh = convert_mesh(&asset.mesh, options)?;
    let mut textures = Vec::new();
    textures
        .try_reserve_exact(asset.textures.len())
        .map_err(|_| SnError::AllocationFailed {
            resource: "prepared textures",
        })?;
    for texture in &asset.textures {
        let prepared = prepare_validated_texture(texture, options)?;
        usage.add_encoded_texture(prepared.png_bytes.len(), &options.limits)?;
        textures.push(prepared);
    }

    Ok(PreparedAsset {
        name: asset.mesh.name.clone(),
        mesh,
        textures,
        materials: asset.materials.clone(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        convert::texture::prepare_texture,
        model::{
            ColorSpace, MeshAsset, MipLevel, Primitive, TextureAsset, TextureFormat,
            VertexAttributes,
        },
    };

    fn triangle() -> Primitive {
        Primitive {
            attributes: VertexAttributes {
                positions: vec![[0.0, 0.0, 0.0], [1.0, 0.0, 0.0], [0.0, 1.0, 0.0]],
                ..VertexAttributes::default()
            },
            indices: vec![0, 1, 2],
            material: None,
        }
    }

    fn mesh(primitive_count: usize) -> MeshAsset {
        MeshAsset {
            name: "sample".to_owned(),
            lods: vec![MeshLod {
                primitives: (0..primitive_count).map(|_| triangle()).collect(),
            }],
            skeleton: None,
        }
    }

    fn texture(name: &str) -> TextureAsset {
        TextureAsset {
            name: name.to_owned(),
            width: 1,
            height: 1,
            format: TextureFormat::Rgba8,
            color_space: ColorSpace::Srgb,
            mips: vec![MipLevel {
                width: 1,
                height: 1,
                data: vec![255, 0, 0, 255],
            }],
        }
    }

    fn asset(primitive_count: usize, texture_count: usize) -> Asset {
        Asset {
            mesh: mesh(primitive_count),
            textures: (0..texture_count)
                .map(|index| texture(&format!("texture_{index}")))
                .collect(),
            materials: Vec::new(),
        }
    }

    #[test]
    fn prepares_multi_primitive_and_multi_texture_asset() {
        let prepared = prepare_asset(&asset(2, 2), &ConversionOptions::default())
            .expect("asset within aggregate budgets must convert");
        assert_eq!(prepared.name, "sample");
        assert_eq!(prepared.mesh.primitives.len(), 2);
        assert_eq!(prepared.textures.len(), 2);
        assert_eq!(
            prepared
                .mesh
                .primitives
                .iter()
                .map(|primitive| primitive.indices.len())
                .sum::<usize>(),
            6
        );
    }

    #[test]
    fn rejects_rgba_aggregated_across_textures_before_encoding() {
        let options = ConversionOptions {
            limits: Limits {
                max_texture_rgba_bytes: 4,
                max_asset_rgba_bytes: 4,
                ..Limits::default()
            },
            ..ConversionOptions::default()
        };
        let error = prepare_asset(&asset(1, 2), &options)
            .expect_err("the second texture must exceed the aggregate RGBA budget");
        assert!(matches!(
            error,
            SnError::LimitExceeded {
                resource: "asset RGBA bytes",
                actual: 8,
                limit: 4
            }
        ));
    }

    #[test]
    fn rejects_png_bytes_aggregated_across_textures() {
        let encoded_len = prepare_texture(&texture("measure"), &ConversionOptions::default())
            .expect("texture must encode")
            .png_bytes
            .len();
        let encoded_limit = u64::try_from(encoded_len).expect("PNG length must fit u64");
        let options = ConversionOptions {
            limits: Limits {
                max_asset_encoded_texture_bytes: encoded_limit,
                ..Limits::default()
            },
            ..ConversionOptions::default()
        };
        let error = prepare_asset(&asset(1, 2), &options)
            .expect_err("the second PNG must exceed the encoded texture budget");
        assert!(matches!(
            error,
            SnError::LimitExceeded {
                resource: "asset encoded texture bytes",
                actual,
                limit
            } if actual == 2 * encoded_limit && limit == encoded_limit
        ));
    }

    #[test]
    fn shared_usage_accumulates_multiple_meshes() {
        let limits = Limits {
            max_vertices: 6,
            max_indices: 6,
            ..Limits::default()
        };
        let mesh = mesh(1).lods.remove(0);
        let mut usage = AssetUsage::default();
        usage.add_mesh(&mesh, &limits).expect("first mesh must fit");
        usage
            .add_mesh(&mesh, &limits)
            .expect("second mesh must fit");
        assert_eq!((usage.vertices, usage.indices), (6, 6));

        let error = usage
            .add_mesh(&mesh, &limits)
            .expect_err("the third mesh must exceed the vertex budget");
        assert!(matches!(
            error,
            SnError::LimitExceeded {
                resource: "vertices",
                actual: 9,
                limit: 6
            }
        ));
    }

    #[test]
    fn shared_usage_enforces_indices_across_meshes() {
        let limits = Limits {
            max_vertices: 9,
            max_indices: 6,
            ..Limits::default()
        };
        let mesh = mesh(1).lods.remove(0);
        let mut usage = AssetUsage::default();
        usage.add_mesh(&mesh, &limits).expect("first mesh must fit");
        usage
            .add_mesh(&mesh, &limits)
            .expect("second mesh must fit");

        let error = usage
            .add_mesh(&mesh, &limits)
            .expect_err("the third mesh must exceed the index budget");
        assert!(matches!(
            error,
            SnError::LimitExceeded {
                resource: "indices",
                actual: 9,
                limit: 6
            }
        ));
    }

    #[test]
    fn detects_aggregate_rgba_overflow() {
        let limits = Limits {
            max_asset_rgba_bytes: u64::MAX,
            ..Limits::default()
        };
        let mut usage = AssetUsage {
            rgba_bytes: u64::MAX - 3,
            ..AssetUsage::default()
        };
        let error = usage
            .add_rgba(4, &limits)
            .expect_err("aggregate RGBA addition must overflow");
        assert!(matches!(
            error,
            SnError::SizeOverflow {
                resource: "asset RGBA bytes"
            }
        ));
    }
}
