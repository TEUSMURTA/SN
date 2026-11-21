//! C ABI for `save_data_names.c`: drop-in replacements with the original names and signatures, generated
//! by rust-anything from `rust-anything.json`. All of the crate's `unsafe` code lives in this file.

pub mod port;

/// `uint8_t save_data_name_index(uint8_t param_1)` (`save_data_names.c:26`).
#[cfg(not(feature = "shadow"))]
// SAFETY: exported under the C name it replaces: link it instead of the original object.
#[unsafe(no_mangle)]
pub extern "C" fn save_data_name_index(param_1: u8) -> u8 {
    port::save_data_name_index(param_1)
}

/// `save_data_name_index` as `rust_anything_save_data_name_index`, for shadow builds beside the original.
// SAFETY: a name no C code defines.
#[unsafe(no_mangle)]
pub extern "C" fn rust_anything_save_data_name_index(param_1: u8) -> u8 {
    port::save_data_name_index(param_1)
}

/// `uint8_t save_data_name_byte(uint8_t param_1, uint8_t offset)` (`save_data_names.c:31`).
#[cfg(not(feature = "shadow"))]
// SAFETY: exported under the C name it replaces: link it instead of the original object.
#[unsafe(no_mangle)]
pub extern "C" fn save_data_name_byte(param_1: u8, offset: u8) -> u8 {
    port::save_data_name_byte(param_1, offset)
}

/// `save_data_name_byte` as `rust_anything_save_data_name_byte`, for shadow builds beside the original.
// SAFETY: a name no C code defines.
#[unsafe(no_mangle)]
pub extern "C" fn rust_anything_save_data_name_byte(param_1: u8, offset: u8) -> u8 {
    port::save_data_name_byte(param_1, offset)
}
