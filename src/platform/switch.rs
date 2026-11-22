use crate::config::{ConversionOptions, Limits, TextureTarget};

/// Nintendo Switch build targeted by this conservative portable profile.
pub const TARGET_ID: &str = "05EC0BBBE4BAE000";

/// Keeps PNG output until KTX2 support is validated with Carafe on [`TARGET_ID`].
#[must_use]
pub fn conversion_options() -> ConversionOptions {
    ConversionOptions {
        limits: Limits {
            max_input_bytes: 64 * 1024 * 1024,
            max_asset_bytes: 64 * 1024 * 1024,
            max_vertices: 250_000,
            max_indices: 1_500_000,
            max_texture_pixels: 4 * 1024 * 1024,
            max_texture_rgba_bytes: 16 * 1024 * 1024,
            max_asset_rgba_bytes: 32 * 1024 * 1024,
            max_asset_encoded_texture_bytes: 32 * 1024 * 1024,
        },
        max_texture_dimension: Some(1024),
        texture_target: TextureTarget::Png,
        ..ConversionOptions::default()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn switch_profile_has_complete_target_budgets() {
        let options = conversion_options();
        assert_eq!(TARGET_ID, "05EC0BBBE4BAE000");
        assert_eq!(
            options.limits,
            Limits {
                max_input_bytes: 64 * 1024 * 1024,
                max_asset_bytes: 64 * 1024 * 1024,
                max_vertices: 250_000,
                max_indices: 1_500_000,
                max_texture_pixels: 4_194_304,
                max_texture_rgba_bytes: 16 * 1024 * 1024,
                max_asset_rgba_bytes: 32 * 1024 * 1024,
                max_asset_encoded_texture_bytes: 32 * 1024 * 1024,
            }
        );
        assert_eq!(options.max_texture_dimension, Some(1024));
        assert_eq!(options.texture_target, TextureTarget::Png);
        assert_eq!(options.lod, 0);
        assert!(!options.flip_v);
        options.validate().expect("Switch limits must be coherent");
    }
}
