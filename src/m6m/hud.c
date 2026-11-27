// LV HUD for grid modes 7..17: level, aspect masks, focus peaking, zebras, RGB histogram, false color (IRE)
#include "dp2m.h"
#include "catalog.h"

#define FB_W 640u
#define LV_H 424u
#define GRID_LEVEL       7u
#define GRID_PEAK        11u
#define GRID_PEAK_LVL    12u
#define GRID_ZEBRA       13u
#define GRID_HIST        14u
#define GRID_FALSE_COLOR 17u
#define GRID_LAST        17u

static int is_frame_grid(uint32_t m) {
    return (m >= GRID_PEAK && m <= GRID_HIST) || m == GRID_FALSE_COLOR;
}

static int is_full_frame_grid(uint32_t m) {
    return m == GRID_PEAK || m == GRID_PEAK_LVL || m == GRID_ZEBRA || m == GRID_FALSE_COLOR;
}

static int plane_ok(uint32_t a) {
    return !(a & 3u) && (a - 0xA0800000u <= 0x0F800000u - FB_W * LV_H);
}

#define RGBA4(r, g, b, a) ((uint16_t)(((r) << 12) | ((g) << 8) | ((b) << 4) | (a)))

// 16-byte aligned zero rects hit the Milbeaut 2D DMA fast path (0xA00B1870) inside stock_fill_rect
static void clear_dma(uint16_t x, uint16_t y, uint16_t w, uint16_t h) {
    const uint16_t rect[4] = { x, y, w, h }, zero[4] = { 0, 0, 0, 0 };
    hw_dcache_sync();
    stock_fill_rect(rect, zero);
}

__attribute__((noinline, target("arm")))
static void fill_argb(uint32_t x, uint32_t y, uint32_t w, uint32_t h, uint16_t px) {
    if (!w || !h) return;
    uint16_t *row = ARGB_FB + y * FB_W + x;
    if (w == 1u) {
        for (uint32_t r = 0; r < h; r++, row += FB_W) *row = px;
        return;
    }
    uint32_t px2 = (uint32_t)px << 16 | px;
    if (w >= 17u && ((uintptr_t)row & 2u)) {
        for (uint32_t r = 0; r < h; r++) row[r * FB_W] = px;
        row++; x++; w--;
    }
    if (w >= 16u && !((uintptr_t)row & 2u)) {
        /* 8-word stmia bursts a full 32 B ARM926 cache line; load fill regs once across all rows */
        uint32_t chunks = w >> 4, rows = h;
        uint32_t step = (FB_W - (chunks << 4)) * 2u;
        __asm__ volatile(
            "mov r4, %3\n\t"
            "mov r5, %3\n\t"
            "mov r6, %3\n\t"
            "mov r7, %3\n\t"
            "mov r8, %3\n\t"
            "mov r9, %3\n\t"
            "mov r10, %3\n\t"
            "mov ip, %3\n\t"
            "1:\n\t"
            "mov r2, %2\n\t"
            "2:\n\t"
            "stmia %0!, {r4-r10, ip}\n\t"
            "subs r2, r2, #1\n\t"
            "bne 2b\n\t"
            "add %0, %0, %4\n\t"
            "subs %1, %1, #1\n\t"
            "bne 1b\n\t"
            : "+r"(row), "+r"(rows)
            : "r"(chunks), "r"(px2), "r"(step)
            : "r2", "r4", "r5", "r6", "r7", "r8", "r9", "r10", "ip", "cc", "memory");
        if (!(w & 15u)) return;
        row = ARGB_FB + y * FB_W + x + (chunks << 4);
        w &= 15u;
    }
    for (uint32_t r = 0; r < h; r++, row += FB_W) {
        uint16_t *p = row;
        uint32_t n = w;
        if (n && ((uintptr_t)p & 2u)) { *p++ = px; n--; }
        uint32_t *pw = (uint32_t *)(void *)p;
        for (; n >= 2u; n -= 2u) *pw++ = px2;
        if (n) *(uint16_t *)(void *)pw = px;
    }
}

struct tilt { uint32_t portrait; int32_t slope; }; /* slope q6, |slope| <= 38 */

static struct tilt read_tilt(void) {
    int32_t ax = (int8_t)stock_accel_x(), ay = (int8_t)stock_accel_y();
    int32_t abs_x = ax < 0 ? -ax : ax, abs_y = ay < 0 ? -ay : ay;
    uint32_t portrait = abs_x > abs_y;
    int32_t num = portrait ? ay : ax, den = portrait ? abs_x : abs_y;
    if (den < 16) den = 16;
    int32_t neg = num < 0, q = 0;
    num = (neg ? -num : num) << 6;
    for (int32_t bit = 6; bit >= 0; bit--)
        if (num >= den << bit) { num -= den << bit; q |= 1 << bit; }
    if (q > 38) q = 38;
    return (struct tilt){ portrait, neg ? -q : q };
}

// widget 0x14 dirty key: tilt for level modes, half-rate frame count for per-frame overlays
uint32_t grid_widget_get(void) {
    uint32_t m = stock_get_grid();
    if (m - GRID_LEVEL > GRID_LAST - GRID_LEVEL || stock_ui_screen_id() != SCREEN_LV || stock_lv_active() != 1u) return m;
    if (is_frame_grid(m)) return m << 16 | ((lv_planes.fcnt >> 1) & 0xFFFFu);
    struct tilt t = read_tilt();
    int32_t s = (t.slope >= -1 && t.slope <= 1) ? 0 : t.slope;
    return m << 16 | t.portrait << 8 | (uint32_t)((s + 64) & 0xFF);
}
HOOK_WORD(0xA07FA4A4, grid_widget_get, 0xA0167F99);

static void draw_level(void) {
    struct tilt t = read_tilt();
    int level = t.slope >= -2 && t.slope <= 2;
    uint16_t fg = level ? RGBA4(1, 15, 3, 14) : RGBA4(15, 15, 15, 14);
    if (!t.portrait) {
        fill_argb(210, 210, 24, 5, 0x000Au); fill_argb(211, 211, 22, 3, fg);
        fill_argb(406, 210, 24, 5, 0x000Au); fill_argb(407, 211, 22, 3, fg);
        fill_argb(318, 207, 5, 11, 0x0009u); fill_argb(319, 208, 3, 9, fg);
    } else {
        fill_argb(318, 108, 5, 24, 0x000Au); fill_argb(319, 109, 3, 22, fg);
        fill_argb(318, 292, 5, 24, 0x000Au); fill_argb(319, 293, 3, 22, fg);
        fill_argb(315, 210, 11, 5, 0x0009u); fill_argb(316, 211, 9, 3, fg);
    }
    int32_t span = t.portrait ? 68 : 72;
    for (int32_t d = -span; d < span; d += 4) {
        if (d >= -8 && d < 8) continue;
        int32_t o = ((d + 2) * t.slope) >> 6;
        o = o < -54 ? -54 : o > 54 ? 54 : o;
        if (!t.portrait) {
            uint32_t px = (uint32_t)(320 + d), py = (uint32_t)(211 + o);
            fill_argb(px, py - 1, 4, 5, 0x0009u); fill_argb(px, py, 4, 3, fg);
        } else {
            uint32_t px = (uint32_t)(319 + o), py = (uint32_t)(212 + d);
            fill_argb(px - 1, py, 5, 4, 0x0009u); fill_argb(px, py, 3, 4, fg);
        }
    }
    stock_flush_rect(206, 107, 434, 316);
}

__attribute__((always_inline, target("arm")))
static inline void fill_col(uint32_t **col, uint32_t n, uint32_t px) {
    if (!n) return;
    uint32_t px2 = (px << 16) | px;
    uint32_t *p = *col;
    do {
        *p = px2;
        p -= FB_W / 2u;
    } while (--n);
    *col = p;
}

__attribute__((noinline, target("arm")))
static void draw_histogram_arm(const uint8_t *Yp, const uint8_t *Cb, const uint8_t *Cr) {
    uint16_t hr[64] = { 0 }, hg[64] = { 0 }, hb[64] = { 0 };
    if (Cb && Cr) {
        const uint8_t *yr = Yp + 4u * FB_W + 4u;
        const uint8_t *cbr = Cb + 4u * 320u + 2u;
        const uint8_t *crr = Cr + 4u * 320u + 2u;
        uint32_t k_rb = (454u << 16) | 359u;
        uint32_t k_g = (183u << 16) | 88u;
        for (uint32_t y = 52u; y; y--, yr += 8u * FB_W - 79u * 8u, cbr += 8u * 320u - 79u * 4u, crr += 8u * 320u - 79u * 4u) {
            uint32_t x = 79u;
            __asm__ volatile(
                "1:\n\t"
                "ldrb r0, [%[yr]], #8\n\t"
                "ldrb r2, [%[crr]], #4\n\t"
                "sub r2, r2, #128\n\t"
                "smulbb r3, r2, %[krb]\n\t"
                "add r3, r0, r3, asr #8\n\t"
                "cmp r3, #255\n\t"
                "andls r3, %[m126], r3, lsr #1\n\t"
                "bichi r3, %[m126], r3, asr #31\n\t"
                "ldrh r1, [%[hr], r3]\n\t"
                "add r1, r1, #1\n\t"
                "strh r1, [%[hr], r3]\n\t"
                "ldrb r1, [%[cbr]], #4\n\t"
                "sub r1, r1, #128\n\t"
                "smulbb r3, r1, %[kg]\n\t"
                "smlabt r3, r2, %[kg], r3\n\t"
                "sub r3, r0, r3, asr #8\n\t"
                "cmp r3, #255\n\t"
                "andls r3, %[m126], r3, lsr #1\n\t"
                "bichi r3, %[m126], r3, asr #31\n\t"
                "ldrh r2, [%[hg], r3]\n\t"
                "add r2, r2, #1\n\t"
                "strh r2, [%[hg], r3]\n\t"
                "smulbt r3, r1, %[krb]\n\t"
                "add r3, r0, r3, asr #8\n\t"
                "cmp r3, #255\n\t"
                "andls r3, %[m126], r3, lsr #1\n\t"
                "bichi r3, %[m126], r3, asr #31\n\t"
                "ldrh r2, [%[hb], r3]\n\t"
                "add r2, r2, #1\n\t"
                "strh r2, [%[hb], r3]\n\t"
                "subs %[x], %[x], #1\n\t"
                "bne 1b\n\t"
                : [yr] "+r"(yr), [cbr] "+r"(cbr), [crr] "+r"(crr), [x] "+r"(x)
                : [hr] "r"(hr), [hg] "r"(hg), [hb] "r"(hb),
                  [krb] "r"(k_rb), [kg] "r"(k_g), [m126] "r"(126u)
                : "r0", "r1", "r2", "r3", "cc", "memory");
        }
    } else {
        const uint8_t *yr = Yp + 4u * FB_W + 4u;
        for (uint32_t y = 52u; y; y--, yr += 8u * FB_W - 79u * 8u) {
            for (uint32_t x = 79u; x; x--, yr += 8) {
                uint32_t bin = *yr >> 2;
                hr[bin]++; hg[bin]++; hb[bin]++;
            }
        }
    }
    uint32_t peak = 24;
    for (uint32_t i = 2; i < 62; i++) {
        if (hr[i] > peak) peak = hr[i];
        if (hg[i] > peak) peak = hg[i];
        if (hb[i] > peak) peak = hb[i];
    }

    /* Exact reciprocal for min(48, (v * 48) / peak) over peak in [24, 4108], v < peak */
    uint32_t rem = (48u << 24) + peak - 1u;
    uint32_t sh = (uint32_t)__builtin_clz(peak) - (uint32_t)__builtin_clz(rem);
    uint32_t d = peak << sh, inv = 0;
    for (uint32_t bit = 1u << sh; bit; bit >>= 1, d >>= 1)
        if (rem >= d) { rem -= d; inv |= bit; }

    enum { HX = 492, HY = 354, HW = 136, HH = 58 };
    fill_argb(HX + 1, HY + 1, HW - 2, HH - 2, 0x000Bu);
    fill_argb(HX, HY, HW, 1, 0xBBBCu);
    fill_argb(HX, HY + HH - 1, HW, 1, 0xBBBCu);
    uint16_t *fb = ARGB_FB;
    uint16_t *row = fb + (HY + 1u) * FB_W + HX;
    for (uint32_t dy = HH - 2u; dy; dy--, row += FB_W) {
        row[0] = 0xBBBCu;
        row[HW - 1] = 0xBBBCu;
    }
    uint16_t *gp = fb + (HY + 2u) * FB_W + HX + 34u;
#pragma clang loop unroll(disable)
    for (uint32_t dy = (HH - 4u) / 2u; dy; dy--, gp += 2u * FB_W) {
        gp[0] = 0x5559u;
        gp[32] = 0x5559u;
        gp[64] = 0x5559u;
    }
    uint32_t *col0 = (uint32_t *)(void *)(fb + (HY + HH - 5u) * FB_W + HX + 4u);
    for (uint32_t i = 0; i < 64; i++, col0++) {
        uint32_t vr = hr[i], vg = hg[i], vb = hb[i];
        uint32_t h0 = vr >= peak ? 48u : (vr * inv) >> 24;
        uint32_t h1 = vg >= peak ? 48u : (vg * inv) >> 24;
        uint32_t h2 = vb >= peak ? 48u : (vb * inv) >> 24;
        if (!(h0 | h1 | h2)) continue;
        uint32_t m0 = 14u << 12, m1 = 14u << 8, m2 = 15u << 4, t;
        if (h0 > h1) { t = h0; h0 = h1; h1 = t; t = m0; m0 = m1; m1 = t; }
        if (h1 > h2) { t = h1; h1 = h2; h2 = t; t = m1; m1 = m2; m2 = t; }
        if (h0 > h1) { t = h0; h0 = h1; h1 = t; t = m0; m0 = m1; m1 = t; }
        uint32_t *col = col0;
        fill_col(&col, h0, m0 | m1 | m2 | 14u);
        fill_col(&col, h1 - h0, m1 | m2 | 14u);
        fill_col(&col, h2 - h1, m2 | 14u);
    }
    if ((uint32_t)hr[63] + hg[63] + hb[63] > 180u) {
        uint16_t *p = fb + (HY + 2u) * FB_W + HX + HW - 3u;
        for (uint32_t dy = HH - 4u; dy; dy--, p += FB_W)
            p[0] = p[1] = 0xF11F;
    }
}

static void draw_histogram(void) {
    uint32_t y = lv_planes.y, cb = lv_planes.cb, cr = lv_planes.cr;
    if (!plane_ok(y)) return;
    draw_histogram_arm((const uint8_t *)y,
                       plane_ok(cb) ? (const uint8_t *)cb : 0,
                       plane_ok(cr) ? (const uint8_t *)cr : 0);
    stock_flush_rect(492, 354, 492 + 136, 354 + 58);
}

static const struct mask { uint8_t grid, vertical, alpha; uint8_t size; } masks[] = {
    { 8, 0, 10, 74 }, { 9, 0, 10, 31 }, { 10, 1, 10, 107 }, { 15, 1, 9, 70 }, { 16, 1, 9, 37 },
};

static void draw_mask(uint32_t m) {
    for (uint32_t i = 0; i < sizeof masks / sizeof *masks; i++) {
        const struct mask *k = &masks[i];
        if (k->grid != m) continue;
        uint32_t s = k->size;
        uint16_t bg = k->alpha, line = RGBA4(15, 15, 15, 12);
        if (k->vertical) {
            fill_argb(0, 0, s, LV_H, bg);        fill_argb(FB_W - s, 0, s, LV_H, bg);
            fill_argb(s, 0, 1, LV_H, line);      fill_argb(FB_W - s - 1, 0, 1, LV_H, line);
            stock_flush_rect(0, 0, (int32_t)(s + 1), LV_H);
            stock_flush_rect((int32_t)(FB_W - s - 1), 0, FB_W, LV_H);
        } else {
            fill_argb(0, 0, FB_W, s, bg);        fill_argb(0, LV_H - s, FB_W, s, bg);
            fill_argb(0, s, FB_W, 1, line);      fill_argb(0, LV_H - s - 1, FB_W, 1, line);
            stock_flush_rect(0, 0, FB_W, (int32_t)(s + 1));
            stock_flush_rect(0, (int32_t)(LV_H - s - 1), FB_W, LV_H);
        }
    }
}

__attribute__((always_inline, target("arm")))
static inline void peak_store(int32_t gx, int32_t gy, uint32_t *o0, uint32_t on, uint32_t thresh) {
    int32_t g = (gx < 0 ? -gx : gx) + (gy < 0 ? -gy : gy);
    if ((uint32_t)g >= thresh) {
        o0[0] = on;
        o0[FB_W / 2u] = on;
    }
}

__attribute__((used, noinline, cold, target("arm")))
static void peaking_pair_slow(const uint8_t * restrict r0, uint32_t * restrict o0a, uint32_t on, uint32_t thresh) {
    const uint8_t * restrict r1 = r0 + 2u * FB_W;
    const uint8_t * restrict r2 = r0 + 4u * FB_W;
    const uint8_t * restrict r3 = r0 + 6u * FB_W;
    uint32_t * restrict o0b = o0a + FB_W;
    /* Adjacent calls share two samples, keeping cross-group bytes live for at most three calls. */
    int32_t c_m = r1[-4], c1 = r1[0], d0 = r2[-2];
    peak_store(c_m - c1, (int32_t)r0[-2] - d0, o0a + 0, on, thresh);       /* A0 */
    int32_t d2 = r2[2];
    peak_store(d0  - d2, c1 - (int32_t)r3[0],  o0b + 1, on, thresh);       /* B1 */
    int32_t c3 = r1[4];
    peak_store(c1  - c3, (int32_t)r0[2] - d2,  o0a + 2, on, thresh);       /* A2 */
    int32_t d_p = r2[6];
    peak_store(d2  - d_p, c3 - (int32_t)r3[4], o0b + 3, on, thresh);       /* B3 */
    int32_t c0 = r1[-2], c2 = r1[2], d1 = r2[0];
    peak_store(c0  - c2, (int32_t)r0[0] - d1,  o0a + 1, on, thresh);       /* A1 */
    int32_t d_m = r2[-4];
    peak_store(d_m - d1, c0 - (int32_t)r3[-2], o0b + 0, on, thresh);       /* B0 */
    int32_t c_p = r1[6], d3 = r2[4];
    peak_store(c2  - c_p,(int32_t)r0[4] - d3,  o0a + 3, on, thresh);       /* A3 */
    peak_store(d1  - d3, c2 - (int32_t)r3[2],  o0b + 2, on, thresh);       /* B2 */
}

// per-frame overlays run in ARM mode on a DMA-zeroed ARGB back-buffer; only active edge/zebra pixels write SDRAM
// 2 output row-pairs (y, y+2) per outer step: shares row(y) & row(y+2) across both horizontal and vertical diffs
// Exact 2-lane SWAR flat-block bound without 0x0100 bias: borrow sets low16 to 0xFFxx, still non-zero under smask
__attribute__((noinline, target("arm")))
static void draw_peaking_arm(const uint8_t *Y, uint32_t on, uint32_t thresh) {
    _Static_assert(((LV_H - 4u) / 2u) % 2u == 0u, "peaking row-pair count must be even");
    if (!thresh) thresh = 1u;
    uint32_t bias, smask;
    if (thresh > 64u)      { bias = 0x00200020u; smask = 0xFFC0FFC0u; }
    else if (thresh > 32u) { bias = 0x00100010u; smask = 0xFFE0FFE0u; }
    else if (thresh > 16u) { bias = 0x00080008u; smask = 0xFFF0FFF0u; }
    else                   { bias = 0x00000000u; smask = 0xFFFFFFFFu; }

    const uint32_t *r0 = (const uint32_t *)(const void *)Y;
    uint32_t *f0a = (uint32_t *)(void *)(ARGB_FB + 2u * FB_W);
    for (uint32_t pairs = (LV_H - 4u) / 4u; pairs; pairs--, r0 += 4u * FB_W / 4u, f0a += 4u * FB_W / 2u) {
        const uint32_t *r1 = r0 + 2u * FB_W / 4u;
        const uint32_t *r2 = r0 + 4u * FB_W / 4u;
        const uint32_t *r3 = r0 + 6u * FB_W / 4u;
        uint32_t *f0b = f0a + FB_W;
        const uint32_t mask = 0x00FF00FFu;

        uint32_t u0 = mask & (r0[0] >> 8), u1 = mask & (r0[1] >> 8);
        uint32_t c0 = mask & (r1[0] >> 8), c1 = mask & (r1[1] >> 8);
        uint32_t d0 = mask & (r2[0] >> 8), d1 = mask & (r2[1] >> 8);
        uint32_t e0 = mask & (r3[0] >> 8), e1 = mask & (r3[1] >> 8);

        /* Left border x = 2, 4 for row A (y) and row B (y+2) */
        peak_store((int32_t)(c0 >> 16)   - (int32_t)(c1 >> 16),   (int32_t)(u0 & 0xFFu) - (int32_t)(d0 & 0xFFu), f0a + 1, on, thresh);
        peak_store((int32_t)(c0 & 0xFFu) - (int32_t)(c1 & 0xFFu), (int32_t)(u1 >> 16)   - (int32_t)(d1 >> 16),   f0a + 2, on, thresh);
        peak_store((int32_t)(d0 >> 16)   - (int32_t)(d1 >> 16),   (int32_t)(c0 & 0xFFu) - (int32_t)(e0 & 0xFFu), f0b + 1, on, thresh);
        peak_store((int32_t)(d0 & 0xFFu) - (int32_t)(d1 & 0xFFu), (int32_t)(c1 >> 16)   - (int32_t)(e1 >> 16),   f0b + 2, on, thresh);

        uint32_t dv0a = (u1 + bias) - d1;
        uint32_t dv0b = (c1 + bias) - e1;
        const uint32_t *p = r0 + 2;
        uint32_t n = (FB_W / 4u - 2u) / 2u; /* 79 blocks -> x = 6, 8, ..., 636 (zero right border) */
        uintptr_t o0_end = (uintptr_t)(f0a + 3u) + n * 16u;
        uint32_t slow_args[3] = { on, (uint32_t)o0_end, thresh };

        register uint32_t r_p     __asm__("r0")  = (uint32_t)(uintptr_t)p;
        register uint32_t r_n     __asm__("r1")  = n;
        register uint32_t r_c1    __asm__("r2")  = c1;
        register uint32_t r_d1    __asm__("r3")  = d1;
        register uint32_t r_dv0a  __asm__("r4")  = dv0a;
        register uint32_t r_dv0b  __asm__("r5")  = dv0b;
        register uint32_t r_mask  __asm__("r6")  = mask;
        register uint32_t r_bias  __asm__("r7")  = bias;
        register uint32_t r_smask __asm__("r8")  = smask;
        register uint32_t r_sargs __asm__("r9")  = (uint32_t)(uintptr_t)slow_args;
        __asm__ volatile(
            "1:\n\t"
            "ldr r10, [r0, #0x500]\n\t"
            "ldr r11, [r0, #0x504]\n\t"
            "and r10, r6, r10, lsr #8\n\t"
            "add r12, r2, r7\n\t"
            "and r2, r6, r11, lsr #8\n\t"
            "sub lr, r12, r10\n\t"
            "add r12, r10, r7\n\t"
            "sub r11, r12, r2\n\t"
            "orr lr, lr, r11\n\t"
            "ldr r11, [r0, #0xf00]\n\t"
            "and r11, r6, r11, lsr #8\n\t"
            "sub r12, r12, r11\n\t"
            "orr lr, lr, r5, lsl #16\n\t"
            "orr lr, lr, r12\n\t"
            "ldr r11, [r0, #0xf04]\n\t"
            "and r11, r6, r11, lsr #8\n\t"
            "add r12, r2, r7\n\t"
            "sub r5, r12, r11\n\t"
            "orr lr, lr, r5\n\t"
            "ldr r10, [r0, #0xa00]\n\t"
            "ldr r11, [r0, #0xa04]\n\t"
            "and r10, r6, r10, lsr #8\n\t"
            "add r12, r3, r7\n\t"
            "and r3, r6, r11, lsr #8\n\t"
            "sub r12, r12, r10\n\t"
            "orr lr, lr, r12\n\t"
            "add r12, r10, r7\n\t"
            "sub r12, r12, r3\n\t"
            "orr lr, lr, r12\n\t"
            "ldmia r0!, {r11, r12}\n\t"
            "and r11, r6, r11, lsr #8\n\t"
            "and r12, r6, r12, lsr #8\n\t"
            "add r11, r11, r7\n\t"
            "sub r11, r11, r10\n\t"
            "orr lr, lr, r4, lsl #16\n\t"
            "orr lr, lr, r11\n\t"
            "add r12, r12, r7\n\t"
            "sub r4, r12, r3\n\t"
            "orr lr, lr, r4\n\t"
            "tst lr, r8\n\t"
            "bne 2f\n\t"
            "3:\n\t"
            "subs r1, r1, #1\n\t"
            "bne 1b\n\t"
            "b 4f\n\t"
            "2:\n\t"
            "stmfd sp!, {r0, r1, r2, r3}\n\t"
            "sub r0, r0, #8\n\t"
            "ldmia r9, {r2, r3, r12}\n\t"
            "sub r1, r3, r1, lsl #4\n\t"
            "mov r3, r12\n\t"
            "bl peaking_pair_slow\n\t"
            "ldmfd sp!, {r0, r1, r2, r3}\n\t"
            "b 3b\n\t"
            "4:\n\t"
            : "+r"(r_p), "+r"(r_n), "+r"(r_c1), "+r"(r_d1), "+r"(r_dv0a), "+r"(r_dv0b)
            : "r"(r_mask), "r"(r_bias), "r"(r_smask), "r"(r_sargs)
            : "r10", "r11", "r12", "lr", "cc", "memory");
    }
}

static void draw_peaking(const uint8_t *Y) {
    static const uint16_t color[4] = { 0xF11E, 0xFE1E, 0xFFFE, 0x111E }; /* red, yellow, white, black */
    uint32_t on = color[REG8(NV_PEAKCOL) & 3u];
    uint32_t sens = REG8(NV_PEAKSENS);
    if (sens > 3u) sens = 0u;
    if (sens == 3u) sens = REG8(MF_SPOT_ACTIVE) ? 1u : 0u; /* Auto: High during 4x/8x MF Magnify, Med at 1x */
    uint32_t thresh = sens == 1u ? 24u : sens == 2u ? 72u : PEAKING_THRESH;
    clear_dma(0, 0, FB_W, LV_H);
    draw_peaking_arm(Y, on | on << 16, thresh);
}

static const uint32_t zebra_stripes_hi[7] = {
    0xF40EF40Eu, 0xF40EF40Eu, 0x111A111Au, 0x111A111Au,
    0xF40EF40Eu, 0xF40EF40Eu, 0x111A111Au,
};
static const uint32_t zebra_stripes_lo[7] = {
    0x1DFE1DFEu, 0x1DFE1DFEu, 0x111A111Au, 0x111A111Au,
    0x1DFE1DFEu, 0x1DFE1DFEu, 0x111A111Au,
};

// 4-word (16 LCD px) SWAR zebra pass on DMA-zeroed ARGB back-buffer: precomputed row stripes + inline stmia/partial
__attribute__((noinline, target("arm")))
static void draw_zebras_arm(const uint8_t *Y, uint32_t phase, uint32_t thresh) {
    const uint32_t *rw = (const uint32_t *)(const void *)Y;
    uint32_t *f0 = (uint32_t *)(void *)ARGB_FB;
    const uint32_t mask = 0x00FF00FFu;
    uint32_t add = 256u - thresh;
    const uint32_t addk = (add << 16) | add;
    for (uint32_t y = 0; y < LV_H / 2u; y++, rw += 2u * FB_W / 4u, f0 += 2u * FB_W / 2u) {
        const uint32_t *sp_row = zebra_stripes_hi + ((y + phase) & 3u);
        register uint32_t r_rp   __asm__("r12") = (uint32_t)(uintptr_t)rw;
        register uint32_t r_o0   __asm__("r11") = (uint32_t)(uintptr_t)f0;
        register uint32_t r_n    __asm__("lr")  = FB_W / 16u;
        register uint32_t r_mask __asm__("r4")  = mask;
        register uint32_t r_addk __asm__("r5")  = addk;
        register uint32_t r_st   __asm__("r6")  = (uint32_t)(uintptr_t)sp_row;
        __asm__ volatile(
            "ldmia r6, {r7, r8, r9, r10}\n\t"
            "1:\n\t"
            "ldmia r12!, {r0, r1, r2, r3}\n\t"
            "and r0, r4, r0, lsr #8\n\t"
            "and r1, r4, r1, lsr #8\n\t"
            "and r2, r4, r2, lsr #8\n\t"
            "and r3, r4, r3, lsr #8\n\t"
            "add r0, r0, r5\n\t"
            "add r1, r1, r5\n\t"
            "add r2, r2, r5\n\t"
            "add r3, r3, r5\n\t"
            "orr r6, r0, r1\n\t"
            "orr r6, r6, r2\n\t"
            "orr r6, r6, r3\n\t"
            "bics r6, r6, r4\n\t"
            "bne 2f\n\t"
            "add r11, r11, #32\n\t"
            "subs lr, lr, #1\n\t"
            "bne 1b\n\t"
            "b 4f\n\t"
            "2:\n\t"
            "and r6, r0, r1\n\t"
            "and r6, r6, r2\n\t"
            "and r6, r6, r3\n\t"
            "bic r6, r6, r4\n\t"
            "eor r6, r6, #0x01000000\n\t"
            "cmp r6, #0x00000100\n\t"
            "bne 3f\n\t"
            "add r6, r11, #0x500\n\t"
            "stmia r11!, {r7, r8, r9, r10}\n\t"
            "stmia r11!, {r7, r8, r9, r10}\n\t"
            "stmia r6!,  {r7, r8, r9, r10}\n\t"
            "stmia r6,   {r7, r8, r9, r10}\n\t"
            "subs lr, lr, #1\n\t"
            "bne 1b\n\t"
            "b 4f\n\t"
            "3:\n\t"
            "tst r0, #0x01000000\n\t"
            "strne r7, [r11, #0]\n\t"
            "strne r7, [r11, #0x500]\n\t"
            "tst r0, #0x00000100\n\t"
            "strne r8, [r11, #4]\n\t"
            "strne r8, [r11, #0x504]\n\t"
            "tst r1, #0x01000000\n\t"
            "strne r9, [r11, #8]\n\t"
            "strne r9, [r11, #0x508]\n\t"
            "tst r1, #0x00000100\n\t"
            "strne r10, [r11, #12]\n\t"
            "strne r10, [r11, #0x50c]\n\t"
            "tst r2, #0x01000000\n\t"
            "strne r7, [r11, #16]\n\t"
            "strne r7, [r11, #0x510]\n\t"
            "tst r2, #0x00000100\n\t"
            "strne r8, [r11, #20]\n\t"
            "strne r8, [r11, #0x514]\n\t"
            "tst r3, #0x01000000\n\t"
            "strne r9, [r11, #24]\n\t"
            "strne r9, [r11, #0x518]\n\t"
            "tst r3, #0x00000100\n\t"
            "strne r10, [r11, #28]\n\t"
            "strne r10, [r11, #0x51c]\n\t"
            "add r11, r11, #32\n\t"
            "subs lr, lr, #1\n\t"
            "bne 1b\n\t"
            "4:\n\t"
            : "+r"(r_rp), "+r"(r_o0), "+r"(r_n), "+r"(r_st)
            : "r"(r_mask), "r"(r_addk)
            : "r0", "r1", "r2", "r3", "r7", "r8", "r9", "r10", "cc", "memory");
    }
}

// Dual highlight (>= hi_thresh, orange) + shadow (<= 12, reverse-diagonal cyan) 4-word SWAR zebra pass
// Scalar reference for the ARM loop below; not emitted.
__attribute__((unused, noinline, target("arm")))
static void draw_zebras_dual_c(const uint8_t *Y, uint32_t phase, uint32_t hi_thresh) {
    uint32_t addh = 256u - hi_thresh;
    uint32_t mask = 0x00FF00FFu, add_hi = (addh << 16) | addh, add_lo = 0x000D000Du, hi = 0x01000100u;
    const uint32_t *rw = (const uint32_t *)(const void *)Y;
    uint32_t *f0 = (uint32_t *)(void *)ARGB_FB;
    for (uint32_t row = 0; row < LV_H / 2u; row++, rw += 2u * FB_W / 4u, f0 += 2u * FB_W / 2u) {
        const uint32_t *sh = zebra_stripes_hi + ((row + phase) & 3u);
        const uint32_t *sl = zebra_stripes_lo + (((LV_H - row) + phase) & 3u);
        const uint32_t *rp = rw;
        uint32_t *o0 = f0;
        for (uint32_t n = FB_W / 16u; n; n--, rp += 4, o0 += 8) {
            uint32_t y0 = mask & (rp[0] >> 8), y1 = mask & (rp[1] >> 8);
            uint32_t y2 = mask & (rp[2] >> 8), y3 = mask & (rp[3] >> 8);
            uint32_t h0 = y0 + add_hi, h1 = y1 + add_hi, h2 = y2 + add_hi, h3 = y3 + add_hi;
            uint32_t l0 = (y0 ^ mask) + add_lo, l1 = (y1 ^ mask) + add_lo;
            uint32_t l2 = (y2 ^ mask) + add_lo, l3 = (y3 ^ mask) + add_lo;
            if (__builtin_expect(((h0 | h1 | h2 | h3 | l0 | l1 | l2 | l3) & hi) == 0u, 1)) continue;
            uint32_t *o1 = o0 + FB_W / 2u;
            if (h0 & 0x01000000u) o0[0] = o1[0] = sh[0]; else if (l0 & 0x01000000u) o0[0] = o1[0] = sl[0];
            if (h0 & 0x00000100u) o0[1] = o1[1] = sh[1]; else if (l0 & 0x00000100u) o0[1] = o1[1] = sl[1];
            if (h1 & 0x01000000u) o0[2] = o1[2] = sh[2]; else if (l1 & 0x01000000u) o0[2] = o1[2] = sl[2];
            if (h1 & 0x00000100u) o0[3] = o1[3] = sh[3]; else if (l1 & 0x00000100u) o0[3] = o1[3] = sl[3];
            if (h2 & 0x01000000u) o0[4] = o1[4] = sh[0]; else if (l2 & 0x01000000u) o0[4] = o1[4] = sl[0];
            if (h2 & 0x00000100u) o0[5] = o1[5] = sh[1]; else if (l2 & 0x00000100u) o0[5] = o1[5] = sl[1];
            if (h3 & 0x01000000u) o0[6] = o1[6] = sh[2]; else if (l3 & 0x01000000u) o0[6] = o1[6] = sl[2];
            if (h3 & 0x00000100u) o0[7] = o1[7] = sh[3]; else if (l3 & 0x00000100u) o0[7] = o1[7] = sl[3];
        }
    }
}

__attribute__((noinline, target("arm")))
static void draw_zebras_dual_arm(const uint8_t *Y, uint32_t phase) {
    uint32_t addh = 256u - ZEBRA_THRESH;
    uint32_t add_hi = (addh << 16) | addh;
    const uint32_t *rw = (const uint32_t *)(const void *)Y;
    uint32_t *f0 = (uint32_t *)(void *)ARGB_FB;
    for (uint32_t row = 0; row < LV_H / 2u; row++, rw += 2u * FB_W / 4u, f0 += 2u * FB_W / 2u) {
        register uint32_t r_rp   __asm__("r12") = (uint32_t)(uintptr_t)rw;
        register uint32_t r_o0   __asm__("r11") = (uint32_t)(uintptr_t)f0;
        register uint32_t r_n    __asm__("lr")  = FB_W / 16u;
        register uint32_t r_mask __asm__("r4")  = 0x00FF00FFu;
        register uint32_t r_addh __asm__("r5")  = add_hi;
        register uint32_t r_addl __asm__("r6")  = add_hi + 0x010C010Cu; /* (268 + addh) - h = 268 - Y */
        register uint32_t r_sh   __asm__("r7")  = (uint32_t)(uintptr_t)(zebra_stripes_hi + ((row + phase) & 3u));
        register uint32_t r_sl   __asm__("r8")  = (uint32_t)(uintptr_t)(zebra_stripes_lo + (((LV_H - row) + phase) & 3u));
        __asm__ volatile(
            "1:\n\t"
            "ldmia r12!, {r0, r1, r2, r3}\n\t"
            "and r0, r4, r0, lsr #8\n\t"
            "and r1, r4, r1, lsr #8\n\t"
            "and r2, r4, r2, lsr #8\n\t"
            "and r3, r4, r3, lsr #8\n\t"
            "add r0, r0, r5\n\t"
            "add r1, r1, r5\n\t"
            "add r2, r2, r5\n\t"
            "add r3, r3, r5\n\t"
            "sub r9, r6, r0\n\t"
            "sub r10, r6, r1\n\t"
            "orr r9, r9, r10\n\t"
            "sub r10, r6, r2\n\t"
            "orr r9, r9, r10\n\t"
            "sub r10, r6, r3\n\t"
            "orr r9, r9, r10\n\t"
            "orr r10, r0, r1\n\t"
            "orr r10, r10, r2\n\t"
            "orr r10, r10, r3\n\t"
            "orr r9, r9, r10\n\t"
            "bics r9, r9, r4\n\t"
            "bne 2f\n\t"
            "add r11, r11, #32\n\t"
            "subs lr, lr, #1\n\t"
            "bne 1b\n\t"
            "b 5f\n\t"
            "2:\n\t"
            "and r9, r0, r1\n\t"
            "and r9, r9, r2\n\t"
            "and r9, r9, r3\n\t"
            "bic r9, r9, r4\n\t"
            "eor r9, r9, #0x01000000\n\t"
            "cmp r9, #0x00000100\n\t"
            "bne 3f\n\t"
            "ldmia r7, {r0, r1, r2, r3}\n\t"
            "add r9, r11, #0x500\n\t"
            "stmia r11!, {r0, r1, r2, r3}\n\t"
            "stmia r11!, {r0, r1, r2, r3}\n\t"
            "stmia r9!, {r0, r1, r2, r3}\n\t"
            "stmia r9, {r0, r1, r2, r3}\n\t"
            "subs lr, lr, #1\n\t"
            "bne 1b\n\t"
            "b 5f\n\t"
            "3:\n\t"
            "bics r10, r10, r4\n\t"
            "bne 4f\n\t"
            "sub r9, r6, r0\n\t"
            "sub r10, r6, r1\n\t"
            "and r9, r9, r10\n\t"
            "sub r10, r6, r2\n\t"
            "and r9, r9, r10\n\t"
            "sub r10, r6, r3\n\t"
            "and r9, r9, r10\n\t"
            "bic r9, r9, r4\n\t"
            "eor r9, r9, #0x01000000\n\t"
            "cmp r9, #0x00000100\n\t"
            "bne 4f\n\t"
            "ldmia r8, {r0, r1, r2, r3}\n\t"
            "add r9, r11, #0x500\n\t"
            "stmia r11!, {r0, r1, r2, r3}\n\t"
            "stmia r11!, {r0, r1, r2, r3}\n\t"
            "stmia r9!, {r0, r1, r2, r3}\n\t"
            "stmia r9, {r0, r1, r2, r3}\n\t"
            "subs lr, lr, #1\n\t"
            "bne 1b\n\t"
            "b 5f\n\t"
            "4:\n\t"
            "sub r9, r6, r0\n\t"
            "tst r0, #0x01000000\n\t"
            "ldrne r10, [r7, #0]\n\t"
            "strne r10, [r11, #0]\n\t"
            "strne r10, [r11, #0x500]\n\t"
            "tst r9, #0x01000000\n\t"
            "ldrne r10, [r8, #0]\n\t"
            "strne r10, [r11, #0]\n\t"
            "strne r10, [r11, #0x500]\n\t"
            "tst r0, #0x00000100\n\t"
            "ldrne r10, [r7, #4]\n\t"
            "strne r10, [r11, #4]\n\t"
            "strne r10, [r11, #0x504]\n\t"
            "tst r9, #0x00000100\n\t"
            "ldrne r10, [r8, #4]\n\t"
            "strne r10, [r11, #4]\n\t"
            "strne r10, [r11, #0x504]\n\t"
            "sub r9, r6, r1\n\t"
            "tst r1, #0x01000000\n\t"
            "ldrne r10, [r7, #8]\n\t"
            "strne r10, [r11, #8]\n\t"
            "strne r10, [r11, #0x508]\n\t"
            "tst r9, #0x01000000\n\t"
            "ldrne r10, [r8, #8]\n\t"
            "strne r10, [r11, #8]\n\t"
            "strne r10, [r11, #0x508]\n\t"
            "tst r1, #0x00000100\n\t"
            "ldrne r10, [r7, #12]\n\t"
            "strne r10, [r11, #12]\n\t"
            "strne r10, [r11, #0x50c]\n\t"
            "tst r9, #0x00000100\n\t"
            "ldrne r10, [r8, #12]\n\t"
            "strne r10, [r11, #12]\n\t"
            "strne r10, [r11, #0x50c]\n\t"
            "sub r9, r6, r2\n\t"
            "tst r2, #0x01000000\n\t"
            "ldrne r10, [r7, #0]\n\t"
            "strne r10, [r11, #16]\n\t"
            "strne r10, [r11, #0x510]\n\t"
            "tst r9, #0x01000000\n\t"
            "ldrne r10, [r8, #0]\n\t"
            "strne r10, [r11, #16]\n\t"
            "strne r10, [r11, #0x510]\n\t"
            "tst r2, #0x00000100\n\t"
            "ldrne r10, [r7, #4]\n\t"
            "strne r10, [r11, #20]\n\t"
            "strne r10, [r11, #0x514]\n\t"
            "tst r9, #0x00000100\n\t"
            "ldrne r10, [r8, #4]\n\t"
            "strne r10, [r11, #20]\n\t"
            "strne r10, [r11, #0x514]\n\t"
            "sub r9, r6, r3\n\t"
            "tst r3, #0x01000000\n\t"
            "ldrne r10, [r7, #8]\n\t"
            "strne r10, [r11, #24]\n\t"
            "strne r10, [r11, #0x518]\n\t"
            "tst r9, #0x01000000\n\t"
            "ldrne r10, [r8, #8]\n\t"
            "strne r10, [r11, #24]\n\t"
            "strne r10, [r11, #0x518]\n\t"
            "tst r3, #0x00000100\n\t"
            "ldrne r10, [r7, #12]\n\t"
            "strne r10, [r11, #28]\n\t"
            "strne r10, [r11, #0x51c]\n\t"
            "tst r9, #0x00000100\n\t"
            "ldrne r10, [r8, #12]\n\t"
            "strne r10, [r11, #28]\n\t"
            "strne r10, [r11, #0x51c]\n\t"
            "add r11, r11, #32\n\t"
            "subs lr, lr, #1\n\t"
            "bne 1b\n\t"
            "5:\n\t"
            : "+r"(r_rp), "+r"(r_o0), "+r"(r_n)
            : "r"(r_mask), "r"(r_addh), "r"(r_addl), "r"(r_sh), "r"(r_sl)
            : "r0", "r1", "r2", "r3", "r9", "r10", "cc", "memory");
    }
}
static void draw_zebras(const uint8_t *Y) {
    clear_dma(0, 0, FB_W, LV_H);
    uint32_t lvl = REG8(NV_ZEBRALVL);
    if (lvl > 3u) lvl = 0u;
    uint32_t phase = lv_planes.fcnt & 7u;
    if (lvl == 3u) {
        draw_zebras_dual_arm(Y, phase);
        return;
    }
    uint32_t thresh = lvl == 1u ? 230u : lvl == 2u ? 252u : ZEBRA_THRESH;
    draw_zebras_arm(Y, phase, thresh);
}

/* 6 Foveon-calibrated IRE exposure zones (pre-doubled RGBA4444 pairs in .lv.fc, 256 B):
 *   0..2   (0..5% IRE)   Violet 0x61FD (crushed shadow / Foveon noise floor)
 *   3..6   (5..11% IRE)  Blue   0x17FB (deep shadow detail)
 *   27..30 (42..48% IRE) Green  0x1E3C (18% middle gray / Zone V)
 *   36..41 (56..66% IRE) Pink   0xF7BC (skin tone / Zone VI)
 *   56..59 (88..94% IRE) Yellow 0xFE1C (Foveon +0.3 EV highlight knee)
 *   60..63 (94..100%)    Red    0xF11E (clipped highlight)
 */
__attribute__((section(".lv.fc")))
static const uint32_t fc_lut32[64] = {
    0x61FD61FDu, 0x61FD61FDu, 0x61FD61FDu, 0x17FB17FBu, 0x17FB17FBu, 0x17FB17FBu, 0x17FB17FBu, 0u,
    0u,          0u,          0u,          0u,          0u,          0u,          0u,          0u,
    0u,          0u,          0u,          0u,          0u,          0u,          0u,          0u,
    0u,          0u,          0u,          0x1E3C1E3Cu, 0x1E3C1E3Cu, 0x1E3C1E3Cu, 0x1E3C1E3Cu, 0u,
    0u,          0u,          0u,          0u,          0xF7BCF7BCu, 0xF7BCF7BCu, 0xF7BCF7BCu, 0xF7BCF7BCu,
    0xF7BCF7BCu, 0xF7BCF7BCu, 0u,          0u,          0u,          0u,          0u,          0u,
    0u,          0u,          0u,          0u,          0u,          0u,          0u,          0u,
    0xFE1CFE1Cu, 0xFE1CFE1Cu, 0xFE1CFE1Cu, 0xFE1CFE1Cu, 0xF11EF11Eu, 0xF11EF11Eu, 0xF11EF11Eu, 0xF11EF11Eu,
};

// 4-word (16 LCD px) false-color pass: skip all-zero LUT blocks and burst-store every word
// in nonzero blocks. The caller's DMA-zeroed ARGB_FB makes writes of zero LUT words exact.
__attribute__((noinline, target("arm"), section(".lv_text.fc")))
static void draw_false_color_arm(const uint8_t *Y) {
    const uint32_t *rw = (const uint32_t *)(const void *)Y;
    uint32_t *f0 = (uint32_t *)(void *)ARGB_FB;
    const uint32_t mask = 0xFCu;
    for (uint32_t y = 0; y < LV_H / 2u; y++, rw += 2u * FB_W / 4u, f0 += 2u * FB_W / 2u) {
        register uint32_t r_rp   __asm__("r11") = (uint32_t)(uintptr_t)rw;
        register uint32_t r_o0   __asm__("r8")  = (uint32_t)(uintptr_t)f0;
        register uint32_t r_n    __asm__("lr")  = FB_W / 16u;
        register uint32_t r_lut  __asm__("r9")  = (uint32_t)(uintptr_t)fc_lut32;
        register uint32_t r_mask __asm__("r10") = mask;
        __asm__ volatile(
            "1:\n\t"
            "ldmia r11!, {r0, r1, r2, r3}\n\t"
            "and r12, r10, r0, lsr #24\n\t"
            "ldr r4, [r9, r12]\n\t"
            "and r12, r10, r0, lsr #8\n\t"
            "ldr r5, [r9, r12]\n\t"
            "and r12, r10, r1, lsr #24\n\t"
            "ldr r6, [r9, r12]\n\t"
            "and r12, r10, r1, lsr #8\n\t"
            "ldr r7, [r9, r12]\n\t"
            "and r12, r10, r2, lsr #24\n\t"
            "ldr r0, [r9, r12]\n\t"
            "and r12, r10, r2, lsr #8\n\t"
            "ldr r1, [r9, r12]\n\t"
            "and r12, r10, r3, lsr #24\n\t"
            "ldr r2, [r9, r12]\n\t"
            "and r12, r10, r3, lsr #8\n\t"
            "ldr r3, [r9, r12]\n\t"
            "orr r12, r4, r5\n\t"
            "orr r12, r12, r6\n\t"
            "orr r12, r12, r7\n\t"
            "orr r12, r12, r0\n\t"
            "orr r12, r12, r1\n\t"
            "orr r12, r12, r2\n\t"
            "orrs r12, r12, r3\n\t"
            "bne 2f\n\t"
            "add r8, r8, #32\n\t"
            "subs lr, lr, #1\n\t"
            "bne 1b\n\t"
            "b 3f\n\t"
            "2:\n\t"
            /* Ascending register lists are output order: r4-r7 then r0-r3. */
            "add r12, r8, #0x500\n\t"
            "stmia r8!, {r4, r5, r6, r7}\n\t"
            "stmia r8!, {r0, r1, r2, r3}\n\t"
            "stmia r12!, {r4, r5, r6, r7}\n\t"
            "stmia r12, {r0, r1, r2, r3}\n\t"
            "subs lr, lr, #1\n\t"
            "bne 1b\n\t"
            "3:\n\t"
            : "+r"(r_rp), "+r"(r_o0), "+r"(r_n)
            : "r"(r_lut), "r"(r_mask)
            : "r0", "r1", "r2", "r3", "r4", "r5", "r6", "r7", "r12", "cc", "memory");
    }
}

__attribute__((noinline, section(".lv_text.fc_ui")))
static void draw_false_color(const uint8_t *Y) {
    static const uint16_t scale[6] = { 0xF11F, 0xFE1F, 0xF7BF, 0x1E3F, 0x17FF, 0x61FF };
    clear_dma(0, 0, FB_W, LV_H);
    draw_false_color_arm(Y);
    /* Compact 6-zone IRE scale bar on the right edge (Red top -> Violet bottom) */
    fill_argb(622, 114, 12, 196, 0x000Du);
    fill_argb(622, 114, 12, 1, 0xBBBCu);
    fill_argb(622, 309, 12, 1, 0xBBBCu);
    for (uint32_t i = 0; i < 6u; i++)
        fill_argb(624, 117u + i * 32u, 8, 30, scale[i]);
}

static void grid_draw(uint32_t m) {
    if (!is_full_frame_grid(m)) {
        clear_dma(200, 102, 240, 220);
        draw_mask(m);
        draw_level();
        if (m == GRID_HIST) draw_histogram();
        hw_dcache_sync();
        return;
    }
    uint32_t y = lv_planes.y;
    if (!plane_ok(y)) return;
    const uint8_t *Yp = (const uint8_t *)y;
    if (m == GRID_ZEBRA) {
        draw_zebras(Yp);
    } else if (m == GRID_FALSE_COLOR) {
        draw_false_color(Yp);
    } else {
        draw_peaking(Yp);
        if (m == GRID_PEAK_LVL) draw_level();
    }
    hw_dcache_sync();
}

void grid_draw_hook(uint32_t arg) {
    static uint8_t prev_grid = 0xFFu;
    uint32_t m = stock_get_grid();
    uint32_t in_lv = stock_ui_screen_id() == SCREEN_LV && stock_lv_active() == 1u;
    if (in_lv && m != prev_grid) {
        if (prev_grid - GRID_LEVEL <= GRID_LAST - GRID_LEVEL && !is_full_frame_grid(m))
            clear_dma(0, 0, FB_W, LV_H);
        prev_grid = (uint8_t)m;
    }
    if (m - GRID_LEVEL > GRID_LAST - GRID_LEVEL) {
        stock_grid_draw(arg);
        return;
    }
    if (in_lv) grid_draw(m);
}
HOOK_CALL(0xA016C708, grid_draw_hook, 0xA004C3EC);
HOOK_CALL(0xA004C5AE, grid_draw_hook, 0xA004C3EC);
