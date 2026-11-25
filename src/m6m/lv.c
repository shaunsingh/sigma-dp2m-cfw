// live view: film CMCM @ live CCMatrix, film tone curve, mono routing & YCC plane capture
#include "dp2m.h"
#include "catalog.h"

struct lv_planes { uint32_t y, cb, cr, fcnt; };
volatile struct lv_planes lv_planes; /* written by lv_ycap_hook */

static struct {
    const void *lut;   /* stock gamma for the current LV mode */
    uint32_t sz;
    uint32_t applied;  /* packed mode the B2Y gamma currently holds */
} gamma;
static uint8_t gamma_buf[1024];

static const struct lv_look *look_of(uint32_t packed) {
    return (packed - FILM_MODE0 <= PACKED_MAX - FILM_MODE0) ? &dp2m_lv_looks[packed - FILM_MODE0] : 0;
}

// knots at x = 0..128 step 16, 192..384 step 64, 512..1023 step 128
static void expand_knots(const uint8_t *k, uint8_t *out) {
    static const uint8_t shift[3] = { 4, 6, 7 }, count[3] = { 8, 4, 5 };
    uint32_t x = 0;
    for (uint32_t seg = 0; seg < 3; seg++) {
        uint32_t sh = shift[seg], n = 1u << sh;
        for (uint32_t i = 0; i < count[seg]; i++, k++) {
            int32_t y0 = k[0], dy = (int32_t)k[1] - y0;
            for (uint32_t s = 0; s < n; s++) out[x++] = (uint8_t)(y0 + ((dy * (int32_t)s + (int32_t)(n >> 1)) >> sh));
        }
    }
}

static void gamma_apply(uint32_t packed) {
    const struct lv_look *l = look_of(packed);
    if (l && (l->flags & LV_KNOTS)) {
        expand_knots(l->knots, gamma_buf);
        stock_b2y_set_gamma(0, gamma_buf, sizeof gamma_buf);
    } else if (gamma.lut) {
        stock_b2y_set_gamma(0, gamma.lut, gamma.sz);
    }
    gamma.applied = packed;
}

// 0xA0039890: stock programs gamma only on LV mode change (ch 0; GMACTL bit 0 stays clear)
void lv_gamma_hook(uint32_t ch, const void *lut, uint32_t sz) {
    (void)ch;
    gamma.lut = lut;
    gamma.sz = sz;
    gamma_apply(packed_cm(stock_get_cm()));
}
HOOK_CALL(0xA0039890, lv_gamma_hook, 0xA009D940);

// 0xA003A1E6: runs on LV mode change or color-dirty (mode/paper setters), so the tone curve follows live
void lv_mono_hook(uint32_t lv_mode, uint32_t cm) {
    uint32_t packed = packed_cm(cm);
    if (cm >= FILM_MODE0) cm = dp2m_films[cm - FILM_MODE0].mono ? 2u : 3u;
    stock_lv_mono(lv_mode, cm);
    if (packed != gamma.applied) gamma_apply(packed);
}
HOOK_CALL(0xA003A1E6, lv_mono_hook, 0xA003989C);

// 0xA003A20A (every refresh): q10 film CMCM x live q7 CCMatrix, BT.601 Y row; mono films write q7 weights directly
void lv_cmcm_hook(uint32_t lv_mode, uint32_t cm) {
    const struct lv_look *l = look_of(packed_cm(cm));
    if (!l || !(l->flags & (LV_CMCM | LV_MONO_Q7))) {
        stock_lv_cmcm(lv_mode, cm);
        return;
    }
    uint16_t m[14] = { [9] = 0x004D, [10] = 0x0096, [11] = 0x001D, [12] = 2u << 8 };
    if (l->flags & LV_MONO_Q7) {
        for (int k = 0; k < 9; k++) m[k] = (uint16_t)l->cmcm[k] & 0x0FFFu;
    } else {
        uint8_t raw[36];
        if (lv_mode == 7u) stock_get_lv_cc7(raw);
        else stock_get_lv_ccn(raw);
        int32_t cc[9];
        for (int k = 0; k < 9; k++) {
            int32_t v = ((uint32_t)raw[10 + 2 * k] << 8 | raw[11 + 2 * k]) & 0x0FFF;
            cc[k] = (v ^ 0x800) - 0x800;
        }
        for (int i = 0; i < 3; i++)
            for (int j = 0; j < 3; j++) {
                const int16_t *r = &l->cmcm[i * 3];
                int32_t v = (r[0] * cc[j] + r[1] * cc[3 + j] + r[2] * cc[6 + j] + 512) >> 10;
                v = v > 2047 ? 2047 : v < -2047 ? -2047 : v;
                m[i * 3 + j] = (uint16_t)v & 0x0FFFu;
            }
    }
    stock_b2y_set_mat(m);
}
HOOK_CALL(0xA003A20A, lv_cmcm_hook, 0xA0039CE2);
