#include <stdint.h>
#include "m6m_addrs.h"

typedef void (*fn_get_cc_t)(void *buf);
typedef void (*fn_set_mat_t)(void *mat_struct);

// q10 cmcm @ q7 ccmatrix + bt.601 Y weights (0x4D, 0x96, 0x1D) -> B2Y
void lv_cmcm(uint32_t lv_mode, uint32_t cm) {
    uint8_t out_struct[28];
    for (int i = 0; i < 28; i++) out_struct[i] = 0;

    uint16_t *out_m = (uint16_t *)(void *)out_struct;
    out_m[9]  = 0x004Du;
    out_m[10] = 0x0096u;
    out_m[11] = 0x001Du;
    out_struct[0x18] = 2u;

    if (cm >= 14u && cm <= 18u) {
        const int16_t *mw = (const int16_t *)(VA_MONO_Q7_TBL + (cm - 14u) * 20u);
        for (int k = 0; k < 9; k++) out_m[k] = (uint16_t)(mw[k] & 0x0FFF);
        ((fn_set_mat_t)SYM_B2Y_SET_MAT)(out_struct);
        return;
    }

    uint32_t idx = cm - 9u;
    if (cm >= 19u && cm <= 38u) {
        idx = 5u + (cm - 19u);
    } else if (cm >= 39u && cm <= 43u) {
        idx = 25u + (cm - 39u);
    }

    uint8_t cc_buf[36];
    if (lv_mode == 7u) ((fn_get_cc_t)(SYM_GET_LV_CC_7 | 1u))(cc_buf);
    else               ((fn_get_cc_t)(SYM_GET_LV_CC_N | 1u))(cc_buf);

    int32_t cc[9];
    const uint16_t *raw_cc = (const uint16_t *)(void *)(cc_buf + 10);
    for (int k = 0; k < 9; k++) {
        int32_t v = (int32_t)(raw_cc[k] & 0x0FFF);
        if (v & 0x0800) v |= (int32_t)0xFFFFF000u;
        cc[k] = v;
    }

    const int16_t *mat = (const int16_t *)(VA_CMCM_PACKED_30 + idx * 18u);
    for (int i = 0; i < 3; i++) {
        for (int j = 0; j < 3; j++) {
            int32_t acc = (int32_t)mat[i * 3 + 0] * cc[j]
                        + (int32_t)mat[i * 3 + 1] * cc[3 + j]
                        + (int32_t)mat[i * 3 + 2] * cc[6 + j];
            int32_t v = (acc + 512) >> 10;
            if (v > 2047) v = 2047;
            if (v < -2047) v = -2047;
            out_m[i * 3 + j] = (uint16_t)(v & 0x0FFF);
        }
    }
    ((fn_set_mat_t)SYM_B2Y_SET_MAT)(out_struct);
}
