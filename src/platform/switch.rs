use crate::config::{BasisMode, ConversionOptions, Limits, TextureTarget};
use crate::model::GpuTextureFormat;
use crate::platform::Platform;

/// Nintendo Switch build targeted by this conservative portable profile.
pub const TARGET_ID: &str = "05EC0BBBE4BAE000";

/// Formato de GPU do payload neste perfil: PNG→RGBA8, KTX2→ASTC 4x4.
#[must_use]
pub fn gpu_format(target: TextureTarget) -> GpuTextureFormat {
    match target {
        TextureTarget::Png => GpuTextureFormat::Rgba8,
        TextureTarget::Ktx2Basis => GpuTextureFormat::Astc4x4,
    }
}

/// Keeps PNG output until KTX2 support is validated with Carafe on [`TARGET_ID`].
#[must_use]
pub fn conversion_options() -> ConversionOptions {
    ConversionOptions {
        platform: Platform::Switch,
        limits: Limits {
            max_input_bytes: 64 * 1024 * 1024,
            max_asset_bytes: 64 * 1024 * 1024,
            max_vertices: 250_000,
            max_indices: 1_500_000,
            max_texture_pixels: 4 * 1024 * 1024,
            max_texture_rgba_bytes: 16 * 1024 * 1024,
            max_asset_rgba_bytes: 32 * 1024 * 1024,
            max_asset_encoded_texture_bytes: 32 * 1024 * 1024,
            max_asset_gpu_texture_bytes: Some(32 * 1024 * 1024),
        },
        max_texture_dimension: Some(1024),
        texture_target: TextureTarget::Png,
        ..ConversionOptions::default()
    }
}

/// Explicit opt-in for the Switch KTX2/ASTC 4x4 pipeline. The pipeline still
/// rejects `TextureTarget::Ktx2Basis` on Switch unless the `switch-ktx2-astc`
/// feature is enabled at compile time; the default profile stays PNG.
#[must_use]
pub fn ktx2_astc_conversion_options() -> ConversionOptions {
    ConversionOptions {
        platform: Platform::Switch,
        texture_target: TextureTarget::Ktx2Basis,
        basis_mode: BasisMode::Uastc,
        ..conversion_options()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn switch_profile_has_complete_target_budgets() {
        let options = conversion_options();
        assert_eq!(TARGET_ID, "05EC0BBBE4BAE000");
        assert_eq!(options.platform, Platform::Switch);
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
                max_asset_gpu_texture_bytes: Some(32 * 1024 * 1024),
            }
        );
        assert_eq!(options.max_texture_dimension, Some(1024));
        assert_eq!(options.texture_target, TextureTarget::Png);
        assert_eq!(options.lod, 0);
        assert!(!options.flip_v);
        assert_eq!(options.gpu_format(), GpuTextureFormat::Rgba8);
        assert_eq!(
            gpu_format(TextureTarget::Ktx2Basis),
            GpuTextureFormat::Astc4x4
        );
        options.validate().expect("Switch limits must be coherent");
    }

    #[test]
    fn ktx2_astc_options_are_explicit_opt_in() {
        let options = ktx2_astc_conversion_options();
        assert_eq!(options.platform, Platform::Switch);
        assert_eq!(options.texture_target, TextureTarget::Ktx2Basis);
        assert_eq!(options.basis_mode, BasisMode::Uastc);
        assert_eq!(options.max_texture_dimension, Some(1024));
        assert_eq!(options.gpu_format(), GpuTextureFormat::Astc4x4);
        options
            .validate()
            .expect("Switch KTX2 options must be coherent");
    }
}
