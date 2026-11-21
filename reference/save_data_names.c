#include <stdint.h>

/* Extracted from PTR_u_SaveFileNameError_1448ecb70 as UTF-16LE strings. */
static const char *const SAVE_DATA_NAMES[0x13] = {
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
};

__declspec(dllexport) uint8_t save_data_name_index(uint8_t param_1)
{
    return param_1 < 0x13 ? param_1 : 0;
}

__declspec(dllexport) uint8_t save_data_name_byte(uint8_t param_1, uint8_t offset)
{
    const char *name = SAVE_DATA_NAMES[save_data_name_index(param_1)];
    uint8_t index = 0;

    while (name[index] != '\0') {
        if (index == offset) {
            return (uint8_t)name[index];
        }
        index++;
    }
    return 0;
}
