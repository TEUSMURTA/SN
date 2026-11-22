//! Platform-specific conversion profiles.
//!
//! [`SWITCH_TARGET_ID`] associates the conservative Switch profile with its target build.

pub mod pc;
pub mod switch;

pub use switch::TARGET_ID as SWITCH_TARGET_ID;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn switch_target_id_is_reexported_with_the_profile() {
        assert_eq!(SWITCH_TARGET_ID, switch::TARGET_ID);
        assert_eq!(SWITCH_TARGET_ID, "05EC0BBBE4BAE000");
    }
}
