#ifndef DP2M_HUD_LEVEL_H
#define DP2M_HUD_LEVEL_H

#include <stdint.h>
#include "m6m_addrs.h"

typedef uint32_t (*fn_u32_t)(void);
typedef void (*fn_fill_t)(const uint16_t *rect, const uint16_t *rgba);

static inline void fill_box(uint16_t x, uint16_t y, uint16_t w, uint16_t h,
                            uint16_t r, uint16_t g, uint16_t b, uint16_t a) {
    uint16_t rc[4] = { x, y, w, h };
    uint16_t cl[4] = { r, g, b, a };
    ((fn_fill_t)(SYM_FILL_RECT | 1u))(rc, cl);
}

static void draw_level_bar(void) {
    int32_t ax = (int32_t)(int8_t)((fn_u32_t)(SYM_ACCEL_X | 1u))();
    int32_t ay = (int32_t)(int8_t)((fn_u32_t)(SYM_ACCEL_Y | 1u))();
    int32_t abs_x = ax < 0 ? -ax : ax;
    int32_t abs_y = ay < 0 ? -ay : ay;
    int32_t portrait = (abs_x > abs_y) ? 1 : 0;
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
    int32_t is_level = (slope >= -2 && slope <= 2);
    uint16_t cr = is_level ? 1u : 15u;
    uint16_t cg = 15u;
    uint16_t cb = is_level ? 3u : 15u;
    uint16_t ca = 14u;

    if (!portrait) {
        fill_box(210u, 210u, 24u,  5u, 0u, 0u, 0u, 10u);
        fill_box(211u, 211u, 22u,  3u, cr, cg, cb, ca);
        fill_box(406u, 210u, 24u,  5u, 0u, 0u, 0u, 10u);
        fill_box(407u, 211u, 22u,  3u, cr, cg, cb, ca);
        fill_box(318u, 207u,  5u, 11u, 0u, 0u, 0u,  9u);
        fill_box(319u, 208u,  3u,  9u, cr, cg, cb, ca);
        for (int32_t dx = -72; dx < 72; dx += 4) {
            if (dx >= -8 && dx < 8) continue;
            int32_t dy = ((dx + 2) * slope) >> 6;
            if (dy < -54) dy = -54;
            if (dy > 54) dy = 54;
            uint16_t px = (uint16_t)(320 + dx);
            uint16_t py = (uint16_t)(211 + dy);
            fill_box(px, (uint16_t)(py - 1u), 4u, 5u, 0u, 0u, 0u, 9u);
            fill_box(px, py,                  4u, 3u, cr, cg, cb, ca);
        }
    } else {
        fill_box(318u, 108u,  5u, 24u, 0u, 0u, 0u, 10u);
        fill_box(319u, 109u,  3u, 22u, cr, cg, cb, ca);
        fill_box(318u, 292u,  5u, 24u, 0u, 0u, 0u, 10u);
        fill_box(319u, 293u,  3u, 22u, cr, cg, cb, ca);
        fill_box(315u, 210u, 11u,  5u, 0u, 0u, 0u,  9u);
        fill_box(316u, 211u,  9u,  3u, cr, cg, cb, ca);
        for (int32_t dy = -68; dy < 68; dy += 4) {
            if (dy >= -8 && dy < 8) continue;
            int32_t dx = ((dy + 2) * slope) >> 6;
            if (dx < -54) dx = -54;
            if (dx > 54) dx = 54;
            uint16_t px = (uint16_t)(319 + dx);
            uint16_t py = (uint16_t)(212 + dy);
            fill_box((uint16_t)(px - 1u), py, 5u, 4u, 0u, 0u, 0u, 9u);
            fill_box(px,                  py, 3u, 4u, cr, cg, cb, ca);
        }
    }
}

#endif
