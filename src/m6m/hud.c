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

static int plane_ok(uint32_t a) { return a - 0xA0800000u <= 0x0F800000u; }

static void box(uint16_t x, uint16_t y, uint16_t w, uint16_t h, uint16_t r, uint16_t g, uint16_t b, uint16_t a) {
    const uint16_t rect[4] = { x, y, w, h }, rgba[4] = { r, g, b, a };
    stock_fill_rect(rect, rgba);
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
    uint16_t cr = level ? 1 : 15, cb = level ? 3 : 15;
    if (!t.portrait) {
        box(210, 210, 24, 5, 0, 0, 0, 10); box(211, 211, 22, 3, cr, 15, cb, 14);
        box(406, 210, 24, 5, 0, 0, 0, 10); box(407, 211, 22, 3, cr, 15, cb, 14);
        box(318, 207, 5, 11, 0, 0, 0, 9);  box(319, 208, 3, 9, cr, 15, cb, 14);
    } else {
        box(318, 108, 5, 24, 0, 0, 0, 10); box(319, 109, 3, 22, cr, 15, cb, 14);
        box(318, 292, 5, 24, 0, 0, 0, 10); box(319, 293, 3, 22, cr, 15, cb, 14);
        box(315, 210, 11, 5, 0, 0, 0, 9);  box(316, 211, 9, 3, cr, 15, cb, 14);
    }
    int32_t span = t.portrait ? 68 : 72;
    for (int32_t d = -span; d < span; d += 4) {
        if (d >= -8 && d < 8) continue;
        int32_t o = ((d + 2) * t.slope) >> 6;
        o = o < -54 ? -54 : o > 54 ? 54 : o;
        if (!t.portrait) {
            uint16_t px = (uint16_t)(320 + d), py = (uint16_t)(211 + o);
            box(px, py - 1, 4, 5, 0, 0, 0, 9); box(px, py, 4, 3, cr, 15, cb, 14);
        } else {
            uint16_t px = (uint16_t)(319 + o), py = (uint16_t)(212 + d);
            box(px - 1, py, 5, 4, 0, 0, 0, 9); box(px, py, 3, 4, cr, 15, cb, 14);
        }
    }
}

// (v * 48) / peak without __aeabi_uidiv
static uint32_t scale48(uint32_t v, uint32_t peak) {
    uint32_t num = v * 48u, q = 0;
    for (int32_t b = 5; b >= 0; b--)
        if (num >= peak << b) { num -= peak << b; q |= 1u << b; }
    return q > 48u ? 48u : q;
}

static void draw_histogram(void) {
    if (!plane_ok(lv_planes.y)) return;
    const uint8_t *Yp = (const uint8_t *)lv_planes.y;
    const uint8_t *Cb = plane_ok(lv_planes.cb) ? (const uint8_t *)lv_planes.cb : 0;
    const uint8_t *Cr = plane_ok(lv_planes.cr) ? (const uint8_t *)lv_planes.cr : 0;
    uint16_t h[3][64] = { { 0 } };
    for (uint32_t y = 4; y < 420; y += 8) {
        const uint8_t *yr = Yp + y * FB_W;
        for (uint32_t x = 4; x < 636; x += 8) {
            int32_t v = yr[x], rgb[3] = { v, v, v };
            if (Cb && Cr) {
                int32_t cb = Cb[y * 320u + (x >> 1)] - 128, cr = Cr[y * 320u + (x >> 1)] - 128;
                rgb[0] = v + ((359 * cr) >> 8);
                rgb[1] = v - ((88 * cb + 183 * cr) >> 8);
                rgb[2] = v + ((454 * cb) >> 8);
            }
            for (int c = 0; c < 3; c++) h[c][(uint32_t)(rgb[c] < 0 ? 0 : rgb[c] > 255 ? 255 : rgb[c]) >> 2]++;
        }
    }
    uint32_t peak = 24;
    for (int c = 0; c < 3; c++)
        for (uint32_t i = 2; i < 62; i++)
            if (h[c][i] > peak) peak = h[c][i];

    enum { HX = 492, HY = 354, HW = 136, HH = 58 };
    uint16_t *fb = ARGB_FB;
    for (uint32_t dy = 0; dy < HH; dy++) {
        uint16_t *row = fb + (HY + dy) * FB_W + HX;
        uint16_t bg = (dy == 0 || dy == HH - 1) ? 0xBBBC : 0x000B;
        for (uint32_t dx = 0; dx < HW; dx++) row[dx] = (dx == 0 || dx == HW - 1) ? 0xBBBC : bg;
    }
    for (uint32_t gx = 34; gx < 130; gx += 32)
        for (uint32_t dy = 2; dy < HH - 2; dy += 2) fb[(HY + dy) * FB_W + HX + gx] = 0x5559;
    for (uint32_t i = 0; i < 64; i++) {
        uint32_t r = scale48(h[0][i], peak), g = scale48(h[1][i], peak), b = scale48(h[2][i], peak);
        uint32_t top = r > g ? (r > b ? r : b) : (g > b ? g : b);
        uint32_t *col = (uint32_t *)(void *)(fb + (HY + HH - 4) * FB_W + HX + 4 + (i << 1));
        for (uint32_t k = 1; k <= top; k++) {
            uint32_t px = (k <= r ? 14u << 12 : 0) | (k <= g ? 14u << 8 : 0) | (k <= b ? 15u << 4 : 0) | 14u;
            col[-(int32_t)(k * (FB_W / 2))] = px << 16 | px;
        }
    }
    if (h[0][63] + h[1][63] + h[2][63] > 180u)
        for (uint32_t dy = 2; dy < HH - 2; dy++) {
            uint16_t *p = fb + (HY + dy) * FB_W + HX + HW - 3;
            p[0] = p[1] = 0xF11F;
        }
    stock_flush_rect(HX, HY, HX + HW, HY + HH);
}

static const struct mask { uint8_t grid, vertical, alpha; uint8_t size; } masks[] = {
    { 8, 0, 10, 74 }, { 9, 0, 10, 31 }, { 10, 1, 10, 107 }, { 15, 1, 9, 70 }, { 16, 1, 9, 37 },
};

static void draw_mask(uint32_t m) {
    for (uint32_t i = 0; i < sizeof masks / sizeof *masks; i++) {
        const struct mask *k = &masks[i];
        if (k->grid != m) continue;
        uint16_t s = k->size;
        if (k->vertical) {
            box(0, 0, s, LV_H, 0, 0, 0, k->alpha); box(FB_W - s, 0, s, LV_H, 0, 0, 0, k->alpha);
            box(s, 0, 1, LV_H, 15, 15, 15, 12);    box(FB_W - s - 1, 0, 1, LV_H, 15, 15, 15, 12);
        } else {
            box(0, 0, FB_W, s, 0, 0, 0, k->alpha); box(0, LV_H - s, FB_W, s, 0, 0, 0, k->alpha);
            box(0, s, FB_W, 1, 15, 15, 15, 12);    box(0, LV_H - s - 1, FB_W, 1, 15, 15, 15, 12);
        }
    }
}

// per-frame overlays write 2x2 blocks as 32-bit pixel pairs (ARGB4444)
static void draw_peaking(const uint8_t *Y) {
    static const uint16_t color[4] = { 0xF11E, 0xFE1E, 0xFFFE, 0x111E }; /* red, yellow, white, black */
    uint32_t on = color[REG8(NV_PEAKCOL) & 3u];
    on |= on << 16;
    for (uint32_t y = 2; y < LV_H - 2; y += 2) {
        const uint8_t *c = Y + y * FB_W, *u = c - 2 * FB_W, *d = c + 2 * FB_W;
        uint32_t *f0 = (uint32_t *)(void *)(ARGB_FB + y * FB_W), *f1 = f0 + FB_W / 2;
        for (uint32_t x = 2; x < FB_W - 2; x += 2) {
            int32_t gx = c[x + 2] - c[x - 2], gy = d[x] - u[x];
            int32_t g = (gx < 0 ? -gx : gx) + (gy < 0 ? -gy : gy);
            uint32_t px = (uint32_t)g >= PEAKING_THRESH ? on : 0;
            f0[x >> 1] = px;
            f1[x >> 1] = px;
        }
    }
    stock_flush_rect(0, 0, FB_W, LV_H);
}

static void draw_zebras(const uint8_t *Y) {
    uint32_t phase = lv_planes.fcnt & 7u;
    for (uint32_t y = 0; y < LV_H; y += 2) {
        const uint8_t *row = Y + y * FB_W;
        uint32_t *f0 = (uint32_t *)(void *)(ARGB_FB + y * FB_W), *f1 = f0 + FB_W / 2;
        for (uint32_t x = 0; x < FB_W; x += 2) {
            uint32_t px = 0;
            if (row[x] >= ZEBRA_THRESH) px = ((((x + y) >> 1) + phase) & 3u) < 2u ? 0xF40EF40Eu : 0x111A111Au;
            f0[x >> 1] = px;
            f1[x >> 1] = px;
        }
    }
    stock_flush_rect(0, 0, FB_W, LV_H);
}

static void grid_draw(uint32_t m) {
    if (m < GRID_PEAK || m > GRID_ZEBRA) {
        box(204, 102, 232, 220, 0, 0, 0, 0);
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
    uint32_t m = stock_get_grid();
    if (m - GRID_LEVEL > GRID_LAST - GRID_LEVEL) {
        stock_grid_draw(arg);
        return;
    }
    if (stock_ui_screen_id() == SCREEN_LV && stock_lv_active() == 1u) grid_draw(m);
}
HOOK_CALL(0xA016C708, grid_draw_hook, 0xA004C3EC);
HOOK_CALL(0xA004C5AE, grid_draw_hook, 0xA004C3EC);
