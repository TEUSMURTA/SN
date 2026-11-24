use crate::error::{Result, SnError};

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Limits {
    /// Maximum byte length of each source file read from disk.
    pub max_input_bytes: u64,
    /// Maximum byte length of the final GLB, including JSON, BIN and headers.
    pub max_asset_bytes: u64,
    /// Maximum vertices aggregated across primitives in the exported LOD.
    pub max_vertices: usize,
    /// Maximum indices aggregated across primitives in the exported LOD.
    pub max_indices: usize,
    /// Maximum pixels aggregated across the mip chain of one source texture.
    pub max_texture_pixels: u64,
    /// Maximum decoded RGBA bytes aggregated across the mip chain of one texture.
    pub max_texture_rgba_bytes: u64,
    /// Maximum decoded RGBA bytes aggregated across all textures in an asset.
    pub max_asset_rgba_bytes: u64,
    /// Maximum encoded PNG/KTX2 bytes aggregated across all textures in an asset.
    pub max_asset_encoded_texture_bytes: u64,
    /// Maximum declared GPU texture footprint, independent of the GLB byte budget.
    /// `None` disables this budget until a runtime GPU layout is known.
    pub max_asset_gpu_texture_bytes: Option<u64>,
}

impl Default for Limits {
    fn default() -> Self {
        Self {
            max_input_bytes: 256 * 1024 * 1024,
            max_asset_bytes: 256 * 1024 * 1024,
            max_vertices: 1_000_000,
            max_indices: 6_000_000,
            max_texture_pixels: 16 * 1024 * 1024,
            max_texture_rgba_bytes: 64 * 1024 * 1024,
            max_asset_rgba_bytes: 256 * 1024 * 1024,
            max_asset_encoded_texture_bytes: 64 * 1024 * 1024,
            max_asset_gpu_texture_bytes: None,
        }
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum TextureTarget {
    #[default]
    Png,
    Ktx2Basis,
}

/// Compression mode for the PC-only KTX2/Basis prototype. Inert unless
/// [`TextureTarget::Ktx2Basis`] is selected.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum BasisMode {
    Etc1s,
    #[default]
    Uastc,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ConversionOptions {
    pub limits: Limits,
    pub lod: usize,
    pub max_texture_dimension: Option<u32>,
    pub texture_target: TextureTarget,
    pub basis_mode: BasisMode,
    pub flip_v: bool,
}

impl Default for ConversionOptions {
    fn default() -> Self {
        Self {
            limits: Limits::default(),
            lod: 0,
            max_texture_dimension: None,
            texture_target: TextureTarget::Png,
            basis_mode: BasisMode::default(),
            flip_v: false,
        }
    }
}

impl ConversionOptions {
    pub fn validate(&self) -> Result<()> {
        if self.max_texture_dimension == Some(0) {
            return Err(invalid_options(
                "max_texture_dimension must be greater than zero".to_owned(),
            ));
        }

        let limits = &self.limits;
        if limits.max_texture_rgba_bytes > limits.max_asset_rgba_bytes {
            return Err(invalid_options(format!(
                "max_texture_rgba_bytes ({}) exceeds max_asset_rgba_bytes ({})",
                limits.max_texture_rgba_bytes, limits.max_asset_rgba_bytes
            )));
        }
        if limits.max_asset_rgba_bytes > limits.max_asset_bytes {
            return Err(invalid_options(format!(
                "max_asset_rgba_bytes ({}) exceeds max_asset_bytes ({})",
                limits.max_asset_rgba_bytes, limits.max_asset_bytes
            )));
        }
        if limits.max_asset_encoded_texture_bytes > limits.max_asset_bytes {
            return Err(invalid_options(format!(
                "max_asset_encoded_texture_bytes ({}) exceeds max_asset_bytes ({})",
                limits.max_asset_encoded_texture_bytes, limits.max_asset_bytes
            )));
        }
        Ok(())
    }
}

fn invalid_options(reason: String) -> SnError {
    SnError::InvalidData {
        resource: "conversion options",
        reason,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn compact_limits() -> Limits {
        Limits {
            max_input_bytes: 64,
            max_asset_bytes: 32,
            max_vertices: 10,
            max_indices: 30,
            max_texture_pixels: 16,
            max_texture_rgba_bytes: 16,
            max_asset_rgba_bytes: 16,
            max_asset_encoded_texture_bytes: 32,
            max_asset_gpu_texture_bytes: None,
        }
    }

    fn assert_invalid(options: &ConversionOptions, field: &str) {
        let error = options.validate().expect_err("options must be rejected");
        match error {
            SnError::InvalidData {
                resource: "conversion options",
                reason,
            } => assert!(reason.contains(field), "unexpected reason: {reason}"),
            other => panic!("unexpected error: {other:?}"),
        }
    }

    #[test]
    fn default_options_are_coherent() {
        ConversionOptions::default()
            .validate()
            .expect("default options must be valid");
    }

    #[test]
    fn rejects_zero_texture_dimension() {
        let options = ConversionOptions {
            max_texture_dimension: Some(0),
            ..ConversionOptions::default()
        };
        assert_invalid(&options, "max_texture_dimension");
    }

    #[test]
    fn rejects_individual_rgba_limit_above_asset_budget() {
        let options = ConversionOptions {
            limits: Limits {
                max_texture_rgba_bytes: 17,
                ..compact_limits()
            },
            ..ConversionOptions::default()
        };
        assert_invalid(&options, "max_texture_rgba_bytes");
    }

    #[test]
    fn rejects_aggregate_rgba_above_glb_budget() {
        let options = ConversionOptions {
            limits: Limits {
                max_asset_bytes: 8,
                max_asset_encoded_texture_bytes: 8,
                ..compact_limits()
            },
            ..ConversionOptions::default()
        };
        assert_invalid(&options, "max_asset_rgba_bytes");
    }

    #[test]
    fn rejects_encoded_textures_above_glb_budget() {
        let options = ConversionOptions {
            limits: Limits {
                max_asset_bytes: 16,
                max_asset_encoded_texture_bytes: 17,
                ..compact_limits()
            },
            ..ConversionOptions::default()
        };
        assert_invalid(&options, "max_asset_encoded_texture_bytes");
    }

    #[test]
    fn gpu_texture_budget_is_independent_of_glb_budget() {
        let options = ConversionOptions {
            limits: Limits {
                max_asset_bytes: 16,
                max_asset_encoded_texture_bytes: 16,
                max_asset_gpu_texture_bytes: Some(1024),
                ..compact_limits()
            },
            ..ConversionOptions::default()
        };
        options
            .validate()
            .expect("GPU footprint may exceed the encoded GLB budget");
    }

    #[test]
    fn accepts_limits_equal_to_their_aggregate_budgets() {
        let options = ConversionOptions {
            limits: compact_limits(),
            ..ConversionOptions::default()
        };
        options
            .validate()
            .expect("equal individual and aggregate limits must be valid");
    }
}
