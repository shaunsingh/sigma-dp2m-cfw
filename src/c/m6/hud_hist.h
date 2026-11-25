#ifndef DP2M_HUD_HIST_H
#define DP2M_HUD_HIST_H

#include <stdint.h>
#include "m6m_addrs.h"

typedef void (*fn_flush_t)(int32_t x0, int32_t y0, int32_t x1, int32_t y1);

// (v * 48) / peak w/o __aeabi_uidiv
static inline uint32_t scale48_no_div(uint32_t v, uint32_t peak) {
    uint32_t num = (v << 5) + (v << 4), q = 0u;
    for (int32_t b = 5; b >= 0; b--) {
        uint32_t d = peak << b;
        if (num >= d) { num -= d; q |= (1u << b); }
    }
    return (q > 48u) ? 48u : q;
}

static void draw_rgb_histogram(void) {
    uint32_t y_addr  = *(volatile uint32_t *)VA_RAM_LIVE_Y_PTR;
    uint32_t cb_addr = *(volatile uint32_t *)VA_RAM_LIVE_CB_PTR;
    uint32_t cr_addr = *(volatile uint32_t *)VA_RAM_LIVE_CR_PTR;
    if (y_addr < 0xA0800000u || y_addr > 0xB0000000u) return;

    const uint8_t *Yp  = (const uint8_t *)y_addr;
    const uint8_t *Cbp = (cb_addr >= 0xA0800000u && cb_addr <= 0xB0000000u) ? (const uint8_t *)cb_addr : 0;
    const uint8_t *Crp = (cr_addr >= 0xA0800000u && cr_addr <= 0xB0000000u) ? (const uint8_t *)cr_addr : 0;

    uint16_t hr[64], hg[64], hb[64];
    for (uint32_t i = 0; i < 64u; i++) { hr[i] = 0; hg[i] = 0; hb[i] = 0; }

    for (uint32_t y = 4u; y < 420u; y += 8u) {
        const uint8_t *yr  = Yp + y * 640u;
        const uint8_t *cbr = Cbp ? (Cbp + y * 320u) : 0;
        const uint8_t *crr = Crp ? (Crp + y * 320u) : 0;
        for (uint32_t x = 4u; x < 636u; x += 8u) {
            int32_t yv = (int32_t)yr[x], rv = yv, gv = yv, bv = yv;
            if (cbr && crr) {
                int32_t cb = (int32_t)cbr[x >> 1] - 128;
                int32_t cr = (int32_t)crr[x >> 1] - 128;
                rv = yv + ((359 * cr) >> 8);
                gv = yv - (( 88 * cb + 183 * cr) >> 8);
                bv = yv + ((454 * cb) >> 8);
                if (rv < 0) rv = 0; else if (rv > 255) rv = 255;
                if (gv < 0) gv = 0; else if (gv > 255) gv = 255;
                if (bv < 0) bv = 0; else if (bv > 255) bv = 255;
            }
            hr[(uint32_t)rv >> 2]++;
            hg[(uint32_t)gv >> 2]++;
            hb[(uint32_t)bv >> 2]++;
        }
    }

    uint32_t peak = 24u;
    for (uint32_t i = 2u; i < 62u; i++) {
        if (hr[i] > peak) peak = hr[i];
        if (hg[i] > peak) peak = hg[i];
        if (hb[i] > peak) peak = hb[i];
    }

    const uint32_t HX = 492u, HY = 354u, HW = 136u, HH = 58u;
    uint16_t *fb = (uint16_t *)SYM_ARGB_FB;
    for (uint32_t dy = 0; dy < HH; dy++) {
        uint16_t *row = fb + (HY + dy) * 640u + HX;
        uint16_t bg = (dy == 0u || dy == HH - 1u) ? 0xBBBCu : 0x000Bu;
        for (uint32_t dx = 0; dx < HW; dx++) row[dx] = (dx == 0u || dx == HW - 1u) ? 0xBBBCu : bg;
    }
    for (uint32_t gx = 34u; gx < 130u; gx += 32u) {
        for (uint32_t dy = 2u; dy < HH - 2u; dy += 2u) {
            fb[(HY + dy) * 640u + HX + gx] = 0x5559u;
        }
    }
    for (uint32_t i = 0; i < 64u; i++) {
        uint32_t r_h = scale48_no_div(hr[i], peak);
        uint32_t g_h = scale48_no_div(hg[i], peak);
        uint32_t b_h = scale48_no_div(hb[i], peak);
        uint32_t col_x = HX + 4u + (i << 1);
        for (uint32_t row_h = 1u; row_h <= 48u; row_h++) {
            uint16_t cr = (row_h <= r_h) ? 14u : 0u;
            uint16_t cg = (row_h <= g_h) ? 14u : 0u;
            uint16_t cb = (row_h <= b_h) ? 15u : 0u;
            if (cr | cg | cb) {
                uint16_t px = (uint16_t)((cr << 12) | (cg << 8) | (cb << 4) | 14u);
                uint16_t *dst = fb + (HY + HH - 4u - row_h) * 640u + col_x;
                dst[0] = px;
                dst[1] = px;
            }
        }
    }
    if ((hr[63] + hg[63] + hb[63]) > 180u) {
        for (uint32_t dy = 2u; dy < HH - 2u; dy++) {
            fb[(HY + dy) * 640u + HX + HW - 3u] = 0xF11Fu;
            fb[(HY + dy) * 640u + HX + HW - 2u] = 0xF11Fu;
        }
    }
    ((fn_flush_t)(SYM_FLUSH_RECT | 1u))((int32_t)HX, (int32_t)HY, (int32_t)(HX + HW), (int32_t)(HY + HH));
}

#endif
