use crate::config::{BasisMode, ConversionOptions, TextureTarget};
use crate::model::GpuTextureFormat;
use crate::platform::Platform;

/// Formato de GPU do payload neste perfil: PNG→RGBA8, KTX2→UASTC.
#[must_use]
pub fn gpu_format(target: TextureTarget) -> GpuTextureFormat {
    match target {
        TextureTarget::Png => GpuTextureFormat::Rgba8,
        TextureTarget::Ktx2Basis => GpuTextureFormat::Uastc,
    }
}

#[must_use]
pub fn conversion_options() -> ConversionOptions {
    ConversionOptions {
        platform: Platform::Pc,
        ..ConversionOptions::default()
    }
}

/// Explicit opt-in for the PC-only KTX2/Basis prototype. The pipeline still
/// rejects `TextureTarget::Ktx2Basis` unless the `pc-ktx2-basis` feature is
/// enabled at compile time; Switch and default profiles never call this.
#[must_use]
pub fn ktx2_conversion_options(mode: BasisMode) -> ConversionOptions {
    ConversionOptions {
        platform: Platform::Pc,
        texture_target: TextureTarget::Ktx2Basis,
        basis_mode: mode,
        ..ConversionOptions::default()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pc_profile_keeps_full_resolution() {
        let options = conversion_options();
        assert_eq!(options.platform, Platform::Pc);
        assert_eq!(options.max_texture_dimension, None);
        assert_eq!(options.texture_target, TextureTarget::Png);
        assert_eq!(options.gpu_format(), GpuTextureFormat::Rgba8);
    }

    #[test]
    fn ktx2_options_are_explicit_opt_in() {
        let options = ktx2_conversion_options(BasisMode::Uastc);
        assert_eq!(options.platform, Platform::Pc);
        assert_eq!(options.texture_target, TextureTarget::Ktx2Basis);
        assert_eq!(options.basis_mode, BasisMode::Uastc);
        assert_eq!(options.max_texture_dimension, None);
        assert_eq!(options.gpu_format(), GpuTextureFormat::Uastc);
    }

    #[test]
    fn pc_maps_png_to_rgba8_and_ktx2_to_uastc() {
        assert_eq!(gpu_format(TextureTarget::Png), GpuTextureFormat::Rgba8);
        assert_eq!(
            gpu_format(TextureTarget::Ktx2Basis),
            GpuTextureFormat::Uastc
        );
    }
}
