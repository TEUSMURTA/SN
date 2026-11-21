use crate::config::ConversionOptions;

#[must_use]
pub fn conversion_options() -> ConversionOptions {
    ConversionOptions::default()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pc_profile_keeps_full_resolution() {
        assert_eq!(conversion_options().max_texture_dimension, None);
    }
}
