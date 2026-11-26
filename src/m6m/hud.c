// LV HUD for grid modes 7..16: level, aspect masks, focus peaking, zebras, RGB histogram
#include "dp2m.h"
#include "catalog.h"

struct lv_planes { uint32_t y, cb, cr, fcnt; };
extern volatile struct lv_planes lv_planes;

#define FB_W 640u
#define LV_H 424u
#define GRID_LEVEL     7u
#define GRID_PEAK      11u
#define GRID_PEAK_LVL  12u
#define GRID_ZEBRA     13u
#define GRID_HIST      14u
#define GRID_LAST      16u

static int plane_ok(uint32_t a) { return !(a & 3u) && (a - 0xA0800000u <= 0x0F800000u); }

#define RGBA4(r, g, b, a) ((uint16_t)(((r) << 12) | ((g) << 8) | ((b) << 4) | (a)))

// 16-byte aligned zero rects hit the Milbeaut 2D DMA fast path (0xA00B1870) inside stock_fill_rect
static void clear_dma(uint16_t x, uint16_t y, uint16_t w, uint16_t h) {
    const uint16_t rect[4] = { x, y, w, h }, zero[4] = { 0, 0, 0, 0 };
    stock_fill_rect(rect, zero);
}

__attribute__((noinline, target("arm")))
static void fill_argb(uint32_t x, uint32_t y, uint32_t w, uint32_t h, uint16_t px) {
    uint32_t px2 = (uint32_t)px << 16 | px;
    for (uint32_t r = 0; r < h; r++) {
        uint16_t *p = ARGB_FB + (y + r) * FB_W + x;
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
    if (m >= GRID_PEAK && m <= GRID_HIST) return m << 16 | ((lv_planes.fcnt >> 1) & 0xFFFFu);
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
static inline uint32_t scale48(uint32_t v, uint32_t peak) {
    uint32_t num = v * 48u, q = 0;
    for (int32_t b = 5; b >= 0; b--)
        if (num >= peak << b) { num -= peak << b; q |= 1u << b; }
    return q > 48u ? 48u : q;
}

__attribute__((always_inline, target("arm")))
static inline uint32_t clamp6(int32_t v) {
    return (uint32_t)(v < 0 ? 0 : v > 255 ? 255 : v) >> 2;
}

__attribute__((noinline, target("arm")))
static void draw_histogram_arm(const uint8_t *Yp, const uint8_t *Cb, const uint8_t *Cr) {
    uint16_t hr[64] = { 0 }, hg[64] = { 0 }, hb[64] = { 0 };
    if (Cb && Cr) {
        for (uint32_t y = 4; y < 420; y += 8) {
            const uint8_t *yr = Yp + y * FB_W, *cbr = Cb + y * 320u, *crr = Cr + y * 320u;
            for (uint32_t x = 4; x < 636; x += 8) {
                int32_t v = yr[x], cb = (int32_t)cbr[x >> 1] - 128, cr = (int32_t)crr[x >> 1] - 128;
                hr[clamp6(v + ((359 * cr) >> 8))]++;
                hg[clamp6(v - ((88 * cb + 183 * cr) >> 8))]++;
                hb[clamp6(v + ((454 * cb) >> 8))]++;
            }
        }
    } else {
        for (uint32_t y = 4; y < 420; y += 8) {
            const uint8_t *yr = Yp + y * FB_W;
            for (uint32_t x = 4; x < 636; x += 8) {
                uint32_t bin = yr[x] >> 2;
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

    enum { HX = 492, HY = 354, HW = 136, HH = 58 };
    uint16_t *fb = ARGB_FB;
    for (uint32_t dy = 0; dy < HH; dy++) {
        uint32_t *wrow = (uint32_t *)(void *)(fb + (HY + dy) * FB_W + HX);
        if (dy == 0 || dy == HH - 1) {
            for (uint32_t i = 0; i < HW / 2; i++) wrow[i] = 0xBBBCBBBCu;
        } else {
            wrow[0] = 0xBBBC000Bu;
            for (uint32_t i = 1; i < HW / 2 - 1; i++) wrow[i] = 0x000B000Bu;
            wrow[HW / 2 - 1] = 0x000BBBBCu;
        }
    }
    for (uint32_t gx = 34; gx < 130; gx += 32)
        for (uint32_t dy = 2; dy < HH - 2; dy += 2) fb[(HY + dy) * FB_W + HX + gx] = 0x5559;
    for (uint32_t i = 0; i < 64; i++) {
        uint32_t r = scale48(hr[i], peak), g = scale48(hg[i], peak), b = scale48(hb[i], peak);
        uint32_t top = r > g ? (r > b ? r : b) : (g > b ? g : b);
        uint32_t *col = (uint32_t *)(void *)(fb + (HY + HH - 4) * FB_W + HX + 4 + (i << 1));
        for (uint32_t k = 1; k <= top; k++) {
            uint32_t px = (k <= r ? 14u << 12 : 0) | (k <= g ? 14u << 8 : 0) | (k <= b ? 15u << 4 : 0) | 14u;
            col[-(int32_t)(k * (FB_W / 2))] = px << 16 | px;
        }
    }
    if ((uint32_t)hr[63] + hg[63] + hb[63] > 180u)
        for (uint32_t dy = 2; dy < HH - 2; dy++) {
            uint16_t *p = fb + (HY + dy) * FB_W + HX + HW - 3;
            p[0] = p[1] = 0xF11F;
        }
}

static void draw_histogram(void) {
    if (!plane_ok(lv_planes.y)) return;
    const uint8_t *Cb = plane_ok(lv_planes.cb) ? (const uint8_t *)lv_planes.cb : 0;
    const uint8_t *Cr = plane_ok(lv_planes.cr) ? (const uint8_t *)lv_planes.cr : 0;
    draw_histogram_arm((const uint8_t *)lv_planes.y, Cb, Cr);
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
static inline uint32_t peak_px(int32_t gx, int32_t gy, uint32_t on) {
    int32_t g = (gx < 0 ? -gx : gx) + (gy < 0 ? -gy : gy);
    return (uint32_t)g >= PEAKING_THRESH ? on : 0;
}

// per-frame overlays run in ARM mode and load 32-bit BE luma words (2 even samples per word)
__attribute__((noinline, target("arm")))
static void draw_peaking_arm(const uint8_t *Y, uint32_t on) {
    memset(ARGB_FB, 0, 2 * FB_W * sizeof(uint16_t));
    memset(ARGB_FB + (LV_H - 2) * FB_W, 0, 2 * FB_W * sizeof(uint16_t));
    for (uint32_t y = 2; y < LV_H - 2; y += 2) {
        const uint32_t *cw = (const uint32_t *)(const void *)(Y + y * FB_W);
        const uint32_t *uw = cw - (2 * FB_W / 4), *dw = cw + (2 * FB_W / 4);
        uint32_t *f0 = (uint32_t *)(void *)(ARGB_FB + y * FB_W), *f1 = f0 + FB_W / 2;
        uint32_t w0 = cw[0], wc = cw[1], wu0 = uw[0], wd0 = dw[0];
        f0[0] = f1[0] = 0;
        f0[1] = f1[1] = peak_px((int32_t)(wc >> 24) - (int32_t)(w0 >> 24),
                                (int32_t)((wd0 >> 8) & 0xFFu) - (int32_t)((wu0 >> 8) & 0xFFu), on);
        int32_t prev = (int32_t)((w0 >> 8) & 0xFFu);
        for (uint32_t i = 1; i < FB_W / 4 - 1; i++) {
            uint32_t wn = cw[i + 1], wu = uw[i], wd = dw[i];
            int32_t ca = (int32_t)(wc >> 24), cb = (int32_t)((wc >> 8) & 0xFFu);
            int32_t du0 = (int32_t)(wu >> 24), du1 = (int32_t)((wu >> 8) & 0xFFu);
            uint32_t p0 = peak_px(cb - prev, du0 - (int32_t)(wd >> 24), on);
            uint32_t p1 = peak_px(ca - (int32_t)(wn >> 24), du1 - (int32_t)((wd >> 8) & 0xFFu), on);
            f0[2 * i] = p0; f0[2 * i + 1] = p1;
            f1[2 * i] = p0; f1[2 * i + 1] = p1;
            prev = cb;
            wc = wn;
        }
        uint32_t px_last = peak_px((int32_t)((wc >> 8) & 0xFFu) - prev,
                                   (int32_t)(dw[FB_W / 4 - 1] >> 24) - (int32_t)(uw[FB_W / 4 - 1] >> 24), on);
        f0[FB_W / 2 - 2] = f1[FB_W / 2 - 2] = px_last;
        f0[FB_W / 2 - 1] = f1[FB_W / 2 - 1] = 0;
    }
}

static void draw_peaking(const uint8_t *Y) {
    static const uint16_t color[4] = { 0xF11E, 0xFE1E, 0xFFFE, 0x111E }; /* red, yellow, white, black */
    uint32_t on = color[REG8(NV_PEAKCOL) & 3u];
    draw_peaking_arm(Y, on | on << 16);
    stock_flush_rect(0, 0, FB_W, LV_H);
}

// SWAR carry check: for u = (w >> 8) & 0x00FF00FF, (u + k) carries into 0x01000100 iff either even luma >= ZEBRA_THRESH
__attribute__((noinline, target("arm")))
static void draw_zebras_arm(const uint8_t *Y, uint32_t phase) {
    const uint32_t mask = 0x00FF00FFu, addk = ((256u - ZEBRA_THRESH) << 16) | (256u - ZEBRA_THRESH);
    for (uint32_t y = 0; y < LV_H; y += 2) {
        const uint32_t *rw = (const uint32_t *)(const void *)(Y + y * FB_W);
        uint32_t *f0 = (uint32_t *)(void *)(ARGB_FB + y * FB_W), *f1 = f0 + FB_W / 2;
        uint32_t base = (y >> 1) + phase;
        uint32_t s0 = ((base + 0u) & 3u) < 2u ? 0xF40EF40Eu : 0x111A111Au;
        uint32_t s1 = ((base + 1u) & 3u) < 2u ? 0xF40EF40Eu : 0x111A111Au;
        uint32_t s2 = ((base + 2u) & 3u) < 2u ? 0xF40EF40Eu : 0x111A111Au;
        uint32_t s3 = ((base + 3u) & 3u) < 2u ? 0xF40EF40Eu : 0x111A111Au;
        for (uint32_t i = 0; i < FB_W / 4; i += 2) {
            uint32_t w0 = rw[i], w1 = rw[i + 1];
            uint32_t u0 = mask & (w0 >> 8), u1 = mask & (w1 >> 8);
            if (!(((u0 + addk) | (u1 + addk)) & 0x01000100u)) {
                f0[2 * i] = 0; f0[2 * i + 1] = 0; f0[2 * i + 2] = 0; f0[2 * i + 3] = 0;
                f1[2 * i] = 0; f1[2 * i + 1] = 0; f1[2 * i + 2] = 0; f1[2 * i + 3] = 0;
                continue;
            }
            uint32_t p0 = (w0 >> 24) >= ZEBRA_THRESH ? s0 : 0;
            uint32_t p1 = (u0 & 0xFFu) >= ZEBRA_THRESH ? s1 : 0;
            uint32_t p2 = (w1 >> 24) >= ZEBRA_THRESH ? s2 : 0;
            uint32_t p3 = (u1 & 0xFFu) >= ZEBRA_THRESH ? s3 : 0;
            f0[2 * i] = p0;     f0[2 * i + 1] = p1;
            f0[2 * i + 2] = p2; f0[2 * i + 3] = p3;
            f1[2 * i] = p0;     f1[2 * i + 1] = p1;
            f1[2 * i + 2] = p2; f1[2 * i + 3] = p3;
        }
    }
}

static void draw_zebras(const uint8_t *Y) {
    draw_zebras_arm(Y, lv_planes.fcnt & 7u);
    stock_flush_rect(0, 0, FB_W, LV_H);
}

static void grid_draw(uint32_t m) {
    if (m < GRID_PEAK || m > GRID_ZEBRA) {
        clear_dma(200, 102, 240, 220);
        draw_mask(m);
        draw_level();
        if (m == GRID_HIST) draw_histogram();
        return;
    }
    if (!plane_ok(lv_planes.y)) return;
    if (m == GRID_ZEBRA) {
        draw_zebras((const uint8_t *)lv_planes.y);
        return;
    }
    draw_peaking((const uint8_t *)lv_planes.y);
    if (m == GRID_PEAK_LVL) draw_level();
}

void grid_draw_hook(uint32_t arg) {
    static uint8_t prev_grid = 0xFFu;
    uint32_t m = stock_get_grid();
    uint32_t in_lv = stock_ui_screen_id() == SCREEN_LV && stock_lv_active() == 1u;
    if (in_lv && m != prev_grid) {
        if (prev_grid - GRID_LEVEL <= GRID_LAST - GRID_LEVEL && m != GRID_PEAK && m != GRID_PEAK_LVL && m != GRID_ZEBRA)
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
