#include <stdint.h>
#include "m6m_addrs.h"
#include "hud_level.h"
#include "hud_hist.h"

#ifndef PEAKING_THRESH
#define PEAKING_THRESH 44
#endif

#ifndef ZEBRA_THRESH
#define ZEBRA_THRESH 242u
#endif

void grid_draw(void) {
    if (((fn_u32_t)(SYM_UI_SCREEN_ID | 1u))() != 0x15u) return;
    if (((fn_u32_t)(SYM_LV_ACTIVE | 1u))() != 1u) return;
    uint32_t m = ((fn_u32_t)(SYM_GET_GRID | 1u))();
    if (m < 7u || m > 16u) return;

    if (m <= 10u || m >= 14u) {
        fill_box(204u, 102u, 232u, 220u, 0u, 0u, 0u, 0u);
        if (m == 8u) {
            fill_box(0u, 0u, 640u, 74u, 0u, 0u, 0u, 10u);
            fill_box(0u, 350u, 640u, 74u, 0u, 0u, 0u, 10u);
            fill_box(0u, 74u, 640u, 1u, 15u, 15u, 15u, 12u);
            fill_box(0u, 349u, 640u, 1u, 15u, 15u, 15u, 12u);
        } else if (m == 9u) {
            fill_box(0u, 0u, 640u, 31u, 0u, 0u, 0u, 10u);
            fill_box(0u, 393u, 640u, 31u, 0u, 0u, 0u, 10u);
            fill_box(0u, 31u, 640u, 1u, 15u, 15u, 15u, 12u);
            fill_box(0u, 392u, 640u, 1u, 15u, 15u, 15u, 12u);
        } else if (m == 10u) {
            fill_box(0u, 0u, 107u, 424u, 0u, 0u, 0u, 10u);
            fill_box(533u, 0u, 107u, 424u, 0u, 0u, 0u, 10u);
            fill_box(107u, 0u, 1u, 424u, 15u, 15u, 15u, 12u);
            fill_box(532u, 0u, 1u, 424u, 15u, 15u, 15u, 12u);
        } else if (m == 15u) {
            fill_box(0u, 0u, 70u, 424u, 0u, 0u, 0u, 9u);
            fill_box(570u, 0u, 70u, 424u, 0u, 0u, 0u, 9u);
            fill_box(70u, 0u, 1u, 424u, 15u, 15u, 15u, 12u);
            fill_box(569u, 0u, 1u, 424u, 15u, 15u, 15u, 12u);
        } else if (m == 16u) {
            fill_box(0u, 0u, 37u, 424u, 0u, 0u, 0u, 9u);
            fill_box(603u, 0u, 37u, 424u, 0u, 0u, 0u, 9u);
            fill_box(37u, 0u, 1u, 424u, 15u, 15u, 15u, 12u);
            fill_box(602u, 0u, 1u, 424u, 15u, 15u, 15u, 12u);
        }
        draw_level_bar();
        if (m == 14u) draw_rgb_histogram();
        return;
    }

    uint32_t y_addr = *(volatile uint32_t *)VA_RAM_LIVE_Y_PTR;
    if (y_addr < 0xA0800000u || y_addr > 0xB0000000u) return;
    const uint8_t *Y = (const uint8_t *)y_addr;
    uint16_t *fb = (uint16_t *)SYM_ARGB_FB;

    if (m == 11u || m == 12u) {
        // bit-branch avoids .rodata (only .text patched into cave 1)
        uint32_t pc = *(volatile uint8_t *)VA_NVRAM_PEAKCOL;
        uint16_t peak_px = (pc & 1u)
            ? ((pc & 2u) ? 0x111Eu : 0xFE1Eu)
            : ((pc & 2u) ? 0xFFFEu : 0xF11Eu);
        for (uint32_t y = 2u; y < 422u; y += 2u) {
            const uint8_t *row_c = Y + y * 640u;
            const uint8_t *row_u = row_c - 1280u;
            const uint8_t *row_d = row_c + 1280u;
            uint16_t *fb0 = fb + y * 640u;
            uint16_t *fb1 = fb0 + 640u;
            for (uint32_t x = 2u; x < 638u; x += 2u) {
                int32_t gx = (int32_t)row_c[x + 2u] - (int32_t)row_c[x - 2u];
                int32_t gy = (int32_t)row_d[x] - (int32_t)row_u[x];
                if (gx < 0) gx = -gx;
                if (gy < 0) gy = -gy;
                uint16_t px = ((gx + gy) >= PEAKING_THRESH) ? peak_px : 0x0000u;
                fb0[x] = px; fb0[x + 1u] = px;
                fb1[x] = px; fb1[x + 1u] = px;
            }
        }
        ((fn_flush_t)(SYM_FLUSH_RECT | 1u))(0, 0, 640, 424);
        if (m == 12u) draw_level_bar();
    } else {
        uint32_t phase = (*(volatile uint32_t *)VA_RAM_LV_FCNT) & 7u;
        for (uint32_t y = 0u; y < 424u; y += 2u) {
            const uint8_t *row = Y + y * 640u;
            uint16_t *fb0 = fb + y * 640u;
            uint16_t *fb1 = fb0 + 640u;
            for (uint32_t x = 0u; x < 640u; x += 2u) {
                uint16_t px = 0x0000u;
                if (row[x] >= ZEBRA_THRESH) {
                    px = ((((x + y) >> 1) + phase) & 3u) < 2u ? 0xF40Eu : 0x111Au;
                }
                fb0[x] = px; fb0[x + 1u] = px;
                fb1[x] = px; fb1[x + 1u] = px;
            }
        }
        ((fn_flush_t)(SYM_FLUSH_RECT | 1u))(0, 0, 640, 424);
    }
}
