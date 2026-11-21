use crate::config::ConversionOptions;

/// Conservative portable profile. It does not require KTX2 or meshopt support in Carafe.
#[must_use]
pub fn conversion_options() -> ConversionOptions {
    ConversionOptions {
        max_texture_dimension: Some(1024),
        ..ConversionOptions::default()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn switch_profile_caps_texture_dimensions() {
        assert_eq!(conversion_options().max_texture_dimension, Some(1024));
    }
}
