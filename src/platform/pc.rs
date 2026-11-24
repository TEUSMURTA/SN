use crate::config::{BasisMode, ConversionOptions, TextureTarget};

#[must_use]
pub fn conversion_options() -> ConversionOptions {
    ConversionOptions::default()
}

/// Explicit opt-in for the PC-only KTX2/Basis prototype. The pipeline still
/// rejects `TextureTarget::Ktx2Basis` unless the `pc-ktx2-basis` feature is
/// enabled at compile time; Switch and default profiles never call this.
#[must_use]
pub fn ktx2_conversion_options(mode: BasisMode) -> ConversionOptions {
    ConversionOptions {
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
        assert_eq!(conversion_options().max_texture_dimension, None);
        assert_eq!(conversion_options().texture_target, TextureTarget::Png);
    }

    #[test]
    fn ktx2_options_are_explicit_opt_in() {
        let options = ktx2_conversion_options(BasisMode::Uastc);
        assert_eq!(options.texture_target, TextureTarget::Ktx2Basis);
        assert_eq!(options.basis_mode, BasisMode::Uastc);
        assert_eq!(options.max_texture_dimension, None);
    }
}
