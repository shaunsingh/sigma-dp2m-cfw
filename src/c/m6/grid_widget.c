#include <stdint.h>
#include "m6m_addrs.h"

typedef uint32_t (*fn_u32_t)(void);

// widget 0x14 dirty-check key (tilt or half-rate lv fcnt triggers stock redraw)
uint32_t grid_widget_get(void) {
    uint32_t m = ((fn_u32_t)(SYM_GET_GRID | 1u))();
    if (m < 7u || m > 16u) return m;
    if (((fn_u32_t)(SYM_UI_SCREEN_ID | 1u))() != 0x15u) return m;
    if (((fn_u32_t)(SYM_LV_ACTIVE | 1u))() != 1u) return m;

    if (m >= 11u && m <= 14u) {
        uint32_t fcnt = (*(volatile uint32_t *)VA_RAM_LV_FCNT) >> 1;
        return (m << 16) | (fcnt & 0xFFFFu);
    }

    int32_t ax = (int32_t)(int8_t)((fn_u32_t)(SYM_ACCEL_X | 1u))();
    int32_t ay = (int32_t)(int8_t)((fn_u32_t)(SYM_ACCEL_Y | 1u))();
    int32_t abs_x = ax < 0 ? -ax : ax;
    int32_t abs_y = ay < 0 ? -ay : ay;
    uint32_t portrait = (abs_x > abs_y) ? 1u : 0u;
    int32_t num = portrait ? ay : ax;
    int32_t den = portrait ? abs_x : abs_y;
    if (den < 16) den = 16;

    int32_t sign = 1;
    if (num < 0) { sign = -1; num = -num; }
    num <<= 6;
    int32_t q = 0;
    for (int32_t bit = 6; bit >= 0; bit--) {
        if (num >= (den << bit)) {
            num -= (den << bit);
            q |= (1 << bit);
        }
    }
    if (q > 38) q = 38;
    int32_t slope = sign * q;
    if (slope >= -1 && slope <= 1) slope = 0;
    return (m << 16) | (portrait << 8) | (uint32_t)((slope + 64) & 0xFF);
}
