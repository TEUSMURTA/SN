//! Safe Rust equivalent of `FUN_1410beef0` and its save-data name table.

#![forbid(unsafe_code)]

/// Names extracted from `PTR_u_SaveFileNameError_1448ecb70`.
pub const SAVE_DATA_NAMES: [&str; 0x13] = [
    "SaveFileNameError",
    "SAVEDATA_NORMAL",
    "SAVEDATA_NORMAL",
    "SAVEDATA_AUTO",
    "SAVEDATA_SYSTEM",
    "SAVEDATA_RETRY",
    "SAVEDATA_AUTO",
    "SAVEDATA_SYSTEM",
    "SAVEDATA_NORMAL",
    "SAVEDATA_NORMAL",
    "SAVEDATA_NORMAL",
    "SAVE_DBG",
    "SAVE_DBG",
    "SAVEDATA_SYSTEM",
    "SAVEDATA_SYSTEM",
    "SAVEDATA_NORMAL",
    "SAVEDATA_AUTO",
    "SAVEDATA_NORMAL",
    "SAVEDATA_NORMAL",
];

/// Returns the table index used by the original function.
pub fn save_data_name_index(param_1: u8) -> u8 {
    if param_1 < 0x13 { param_1 } else { 0 }
}

/// Looks up a save-data name, falling back to `SaveFileNameError` for invalid indices.
pub fn save_data_name(param_1: u8) -> &'static str {
    SAVE_DATA_NAMES[usize::from(save_data_name_index(param_1))]
}

/// Byte-oriented view used to characterize the original static C strings.
pub fn save_data_name_byte(param_1: u8, offset: u8) -> u8 {
    save_data_name(param_1)
        .as_bytes()
        .get(usize::from(offset))
        .copied()
        .unwrap_or(0)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn index_0_returns_save_file_name_error() {
        assert_eq!(save_data_name_index(0), 0);
        assert_eq!(save_data_name(0), "SaveFileNameError");
    }

    #[test]
    fn index_0x12_is_the_last_valid_entry() {
        assert_eq!(save_data_name_index(0x12), 0x12);
        assert_eq!(save_data_name(0x12), "SAVEDATA_NORMAL");
    }

    #[test]
    fn index_0x13_falls_back_to_entry_0() {
        assert_eq!(save_data_name_index(0x13), 0);
        assert_eq!(save_data_name(0x13), "SaveFileNameError");
    }

    #[test]
    fn index_255_falls_back_to_entry_0() {
        assert_eq!(save_data_name_index(255), 0);
        assert_eq!(save_data_name(255), "SaveFileNameError");
    }

    #[test]
    fn table_preserves_all_extracted_names_in_order() {
        assert_eq!(
            SAVE_DATA_NAMES,
            [
                "SaveFileNameError",
                "SAVEDATA_NORMAL",
                "SAVEDATA_NORMAL",
                "SAVEDATA_AUTO",
                "SAVEDATA_SYSTEM",
                "SAVEDATA_RETRY",
                "SAVEDATA_AUTO",
                "SAVEDATA_SYSTEM",
                "SAVEDATA_NORMAL",
                "SAVEDATA_NORMAL",
                "SAVEDATA_NORMAL",
                "SAVE_DBG",
                "SAVE_DBG",
                "SAVEDATA_SYSTEM",
                "SAVEDATA_SYSTEM",
                "SAVEDATA_NORMAL",
                "SAVEDATA_AUTO",
                "SAVEDATA_NORMAL",
                "SAVEDATA_NORMAL",
            ]
        );
    }
}
