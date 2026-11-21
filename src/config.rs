use crate::error::{Result, SnError};

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Limits {
    pub max_asset_bytes: u64,
    pub max_vertices: usize,
    pub max_indices: usize,
    pub max_texture_pixels: u64,
    pub max_texture_bytes: u64,
}

impl Default for Limits {
    fn default() -> Self {
        Self {
            max_asset_bytes: 256 * 1024 * 1024,
            max_vertices: 1_000_000,
            max_indices: 6_000_000,
            max_texture_pixels: 16 * 1024 * 1024,
            max_texture_bytes: 64 * 1024 * 1024,
        }
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum TextureTarget {
    #[default]
    Png,
    Ktx2Basis,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ConversionOptions {
    pub limits: Limits,
    pub lod: usize,
    pub max_texture_dimension: Option<u32>,
    pub texture_target: TextureTarget,
    pub flip_v: bool,
}

impl Default for ConversionOptions {
    fn default() -> Self {
        Self {
            limits: Limits::default(),
            lod: 0,
            max_texture_dimension: None,
            texture_target: TextureTarget::Png,
            flip_v: false,
        }
    }
}

impl ConversionOptions {
    pub fn validate(&self) -> Result<()> {
        if self.max_texture_dimension == Some(0) {
            return Err(SnError::InvalidData {
                resource: "conversion options",
                reason: "max_texture_dimension must be greater than zero".to_owned(),
            });
        }
        Ok(())
    }
}
