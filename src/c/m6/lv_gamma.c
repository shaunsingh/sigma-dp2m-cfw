#include <stdint.h>
#include "m6m_addrs.h"

typedef uint32_t (*fn_get_cm_t)(void);
typedef void (*fn_set_gam_t)(uint32_t ch, const void *lut, uint32_t sz);

// 18 knots (8@16, 4@64, 5@128) -> 1024B b2y gamma; ch=0 & ctrl_ptr[1]=0 avoids GMACTL bit-0 highlight speckling
static void expand_18_to_1024(const uint8_t *knots, uint8_t *buf) {
    for (uint32_t i = 0; i < 8u; i++) {
        int32_t y0 = (int32_t)knots[i], dy = (int32_t)knots[i + 1u] - y0;
        uint32_t idx = i << 4;
        for (uint32_t s = 0; s < 16u; s++) {
            int32_t v = y0 + ((dy * (int32_t)s + 8) >> 4);
            buf[idx + s] = (uint8_t)(v < 0 ? 0 : (v > 255 ? 255 : v));
        }
    }
    for (uint32_t i = 0; i < 4u; i++) {
        int32_t y0 = (int32_t)knots[8u + i], dy = (int32_t)knots[9u + i] - y0;
        uint32_t idx = 128u + (i << 6);
        for (uint32_t s = 0; s < 64u; s++) {
            int32_t v = y0 + ((dy * (int32_t)s + 32) >> 6);
            buf[idx + s] = (uint8_t)(v < 0 ? 0 : (v > 255 ? 255 : v));
        }
    }
    for (uint32_t i = 0; i < 5u; i++) {
        int32_t y0 = (int32_t)knots[12u + i], dy = (int32_t)knots[13u + i] - y0;
        uint32_t idx = 384u + (i << 7);
        for (uint32_t s = 0; s < 128u; s++) {
            int32_t v = y0 + ((dy * (int32_t)s + 64) >> 7);
            buf[idx + s] = (uint8_t)(v < 0 ? 0 : (v > 255 ? 255 : v));
        }
    }
}

void lv_gamma(uint32_t ch, const void *orig_lut, uint32_t sz, uint8_t *ctrl_ptr) {
    (void)ctrl_ptr;
    uint32_t cm = ((fn_get_cm_t)(SYM_GET_CM | 1u))();
    if (cm >= 19u && cm <= 38u) {
        uint8_t buf[1024];
        const uint8_t *knots = (const uint8_t *)(VA_LV_LUTS_20 + (cm - 19u) * 18u);
        expand_18_to_1024(knots, buf);
        ((fn_set_gam_t)SYM_B2Y_SET_GAMMA)(ch, buf, 1024u);
        return;
    }
    if (cm >= 39u && cm <= 43u) {
        uint8_t buf[1024];
        const uint8_t *knots = (const uint8_t *)(VA_LV_LUTS_BW_5 + (cm - 39u) * 18u);
        expand_18_to_1024(knots, buf);
        ((fn_set_gam_t)SYM_B2Y_SET_GAMMA)(ch, buf, 1024u);
        return;
    }
    ((fn_set_gam_t)SYM_B2Y_SET_GAMMA)(ch, orig_lut, sz);
}