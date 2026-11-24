//! Platform-specific conversion profiles.
//!
//! [`SWITCH_TARGET_ID`] associates the conservative Switch profile with its target build.

pub mod pc;
pub mod switch;

pub use switch::TARGET_ID as SWITCH_TARGET_ID;

/// Platform tag carried by [`crate::config::ConversionOptions`]; each profile
/// in [`pc`] / [`switch`] maps its texture payloads to a GPU format.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Platform {
    #[default]
    Pc,
    Switch,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn switch_target_id_is_reexported_with_the_profile() {
        assert_eq!(SWITCH_TARGET_ID, switch::TARGET_ID);
        assert_eq!(SWITCH_TARGET_ID, "05EC0BBBE4BAE000");
    }

    #[test]
    fn platform_default_is_pc() {
        assert_eq!(Platform::default(), Platform::Pc);
    }
}
