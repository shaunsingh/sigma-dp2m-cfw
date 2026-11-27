// label icons: dp2m strings drawn by the stock font into cached 8bpp tiles in the stock label style
#include "dp2m.h"
#include "catalog.h"

#define ICON_RESULT ((struct icon *)0xA07DD77Cu)
#define LINE_BUF    ((uint8_t *)0xA093A514u) /* stock text scratch: cleared on entry, read only within the call */
#define LB_W        640u
#define FONT_SCALE  0xA07DD774u              /* stock text width scale {num, den}, set by some screens around text */
#define OSD_DIRTY   0xA07DD760u              /* 8bpp OSD dirty rect; the offscreen text blit still widens it */

_Static_assert(ICON_LABEL_L == 1u << 10 && ICON_LABEL_QS == 6u << 10, "label id = class << 10 | string index");

/* tile indices follow font coverage: 1 solid white .. 15 faint, 16 black outline, 0 transparent */
#define OUTLINE  16u
#define INK_MAX  12u /* fainter edge coverage keeps its grey but does not grow the outline */
#define PX_REF   32u
#define PX_MIN   8u
#define PX_MAX   96u /* keeps rows_of(px) inside the 256-row line buffer */
#define PX_RING2 16u /* 2 px outlines from here */
#define PX_BOLD0 14u /* [PX_BOLD0, PX_BOLD1): 1 px heavier strokes; smaller text would close its counters */
#define PX_BOLD1 24u
#define LEN_MAX  48u
#define TILE_MAX (110u * 70u)
#define ARENA    (512u * 1024u)
#define MAGIC    0x4C41424Cu
#define CM_BW    2u
#define CM_VIVID 4u

enum { LEFT = 1, BOLD = 2 }; /* BOLD: wide, heavier tokens like the stock C1..C3 */
static const struct cls { uint8_t w, h, mx, my, hmax, ring, weight, flags; } classes[] = {
    { 110, 70, 3, 2, 36, 2, 8, 0 },   /* ICON_LABEL_L: QS slot value */
    { 54, 42, 2, 2, 24, 1, 8, 0 },    /* ICON_LABEL_S: QS strip, menu row value */
    { 76, 26, 1, 1, 23, 2, 8, LEFT }, /* ICON_LABEL_OSD: LV color mode, left aligned like stock */
    { 51, 31, 2, 1, 29, 2, 9, BOLD }, /* ICON_LABEL_MODE */
    { 35, 20, 1, 0, 19, 1, 9, BOLD }, /* ICON_LABEL_WIDGET */
    { 42, 34, 2, 2, 20, 1, 8, 0 },    /* ICON_LABEL_QS: Custom QS item */
};
#define N_CLASSES (sizeof classes / sizeof *classes)

/* payload .bss is not zeroed: labels_boot resets the cache, the magic covers lookups before it */
static struct {
    uint32_t magic, top;
    uint8_t *tile[N_CLASSES][DP2M_N_STRINGS];
} cache;
static uint8_t arena[ARENA] __attribute__((aligned(4)));
static uint8_t scratch[TILE_MAX] __attribute__((aligned(4))); /* first line of a 2-line label, then the outline mask */
static uint8_t cols[LB_W] __attribute__((aligned(4)));

struct box { uint32_t x0, y0, x1, y1; };

static uint32_t min(uint32_t a, uint32_t b) { return a < b ? a : b; }
static uint32_t max(uint32_t a, uint32_t b) { return a > b ? a : b; }
static uint32_t rows_of(uint32_t px) { return px + (px >> 1) + 12u; }

static uint32_t udiv(uint32_t n, uint32_t d) {
    uint32_t q = 0;
    for (int32_t b = 15; b >= 0; b--)
        if (n >= d << b) { n -= d << b; q |= 1u << b; }
    return q;
}

__attribute__((noinline, target("arm")))
static struct box scan_box(uint32_t rows) {
    struct box b = { 0, 0, 0, 0 };
    if (!rows) return b;
    memset(cols, 0, sizeof cols);
    const uint32_t *rw = (const uint32_t *)(const void *)LINE_BUF;
    uint32_t *ow_base = (uint32_t *)(void *)cols;
    for (uint32_t y = 0; y < rows; y++) {
        uint32_t *ow = ow_base;
        uint32_t nz = 0, n = LB_W / 32u;
        __asm__ volatile(
            "1:\n\t"
            "ldmia %[p]!, {r0, r1, r2, r3, r4, r5, r6, r7}\n\t"
            "orr r12, r0, r1\n\t"
            "orr r12, r12, r2\n\t"
            "orr r12, r12, r3\n\t"
            "orr r12, r12, r4\n\t"
            "orr r12, r12, r5\n\t"
            "orr r12, r12, r6\n\t"
            "orrs r12, r12, r7\n\t"
            "beq 2f\n\t"
            "mov %[nz], #1\n\t"
            "ldr r12, [%[o], #0];  orr r0, r0, r12\n\t"
            "ldr r12, [%[o], #4];  orr r1, r1, r12\n\t"
            "ldr r12, [%[o], #8];  orr r2, r2, r12\n\t"
            "ldr r12, [%[o], #12]; orr r3, r3, r12\n\t"
            "ldr r12, [%[o], #16]; orr r4, r4, r12\n\t"
            "ldr r12, [%[o], #20]; orr r5, r5, r12\n\t"
            "ldr r12, [%[o], #24]; orr r6, r6, r12\n\t"
            "ldr r12, [%[o], #28]; orr r7, r7, r12\n\t"
            "stmia %[o]!, {r0, r1, r2, r3, r4, r5, r6, r7}\n\t"
            "subs %[n], %[n], #1\n\t"
            "bne 1b\n\t"
            "b 3f\n\t"
            "2:\n\t"
            "add %[o], %[o], #32\n\t"
            "subs %[n], %[n], #1\n\t"
            "bne 1b\n\t"
            "3:\n\t"
            : [p] "+r"(rw), [o] "+r"(ow), [n] "+r"(n), [nz] "+r"(nz)
            :
            : "r0", "r1", "r2", "r3", "r4", "r5", "r6", "r7", "r12", "cc", "memory");
        if (nz) {
            if (!b.y1) b.y0 = y;
            b.y1 = y + 1u;
        }
    }
    if (!b.y1) return b;
    uint32_t xw = 0;
    while (!ow_base[xw]) xw++;
    b.x0 = (xw << 2) + ((uint32_t)__builtin_clz(ow_base[xw]) >> 3);
    uint32_t rw_i = LB_W / 4u - 1u;
    while (!ow_base[rw_i]) rw_i--;
    uint32_t w = ow_base[rw_i];
    b.x1 = (rw_i << 2) + ((uint32_t)__builtin_clz(w & (0u - w)) >> 3) + 1u;
    return b;
}

// ink box of s[..n] drawn offscreen (rect y >= 480 leaves the OSD untouched); marks inked columns in cols
static struct box draw(const struct cls *c, const char *s, int32_t n, uint32_t px) {
    static const uint16_t offscreen[4] = { 0, 600, LB_W, 100 };
    stock_text(0, s, n, px, c->weight, 0, !(c->flags & BOLD), offscreen, 0x21, 0);
    return scan_box(rows_of(px));
}

__attribute__((noinline, target("arm")))
static void put(uint8_t *t, const struct cls *c, int32_t x, int32_t y, const uint8_t *src, uint32_t w, uint32_t h, uint32_t stride) {
    int32_t j0 = y < 0 ? -y : 0;
    int32_t j1 = (int32_t)h;
    if (y + j1 > (int32_t)c->h) j1 = (int32_t)c->h - y;
    int32_t i0 = x < 0 ? -x : 0;
    int32_t i1 = (int32_t)w;
    if (x + i1 > (int32_t)c->w) i1 = (int32_t)c->w - x;
    if (j0 >= j1 || i0 >= i1) return;
    uint32_t cw = c->w, len = (uint32_t)(i1 - i0);
    uint8_t *dst = t + (uint32_t)(y + j0) * cw + (uint32_t)(x + i0);
    const uint8_t *sr = src + (uint32_t)j0 * stride + (uint32_t)i0;
    for (int32_t j = j0; j < j1; j++, dst += cw, sr += stride) {
        for (uint32_t i = 0; i < len; i++) {
            uint8_t v = sr[i];
            if (v) dst[i] = v;
        }
    }
}

// MODE / widget tokens & mid-size text: strokes 1 px heavier; stock outline shapes: r = 1 plus, r = 2 5x5 without corners
__attribute__((noinline, target("arm")))
static void finish_tile(uint8_t *t, uint32_t w, uint32_t h, uint32_t bold, uint32_t r, uint8_t *m, struct box ink) {
    uint32_t ix0 = ink.x0, iy0 = ink.y0, ix1 = ink.x1, iy1 = ink.y1;
    if (ix0 >= ix1 || iy0 >= iy1) return;
    if (bold) {
        uint32_t bx0 = ix0 + 1u, bx1 = ix1 < w ? ix1 : w - 1u;
        uint8_t *row = t + iy0 * w;
        for (uint32_t y = iy0; y < iy1; y++, row += w) {
            for (uint32_t x = bx1; x >= bx0; x--) {
                uint8_t l = row[x - 1u];
                if ((uint32_t)(l - 1u) < (uint32_t)(row[x] - 1u)) row[x] = l;
            }
        }
        if (ix1 < w) ix1++;
    }
    static const uint8_t z[112] = { 0 };
    uint32_t oy0 = iy0 > r ? iy0 - r : 0u;
    uint32_t oy1 = iy1 + r < h ? iy1 + r : h;
    uint32_t ox0 = ix0 > r ? ix0 - r : 0u;
    uint32_t ox1 = ix1 + r < w ? ix1 + r : w;
    if (r == 1u) {
        uint8_t *tr = t + oy0 * w;
        for (uint32_t y = oy0; y < oy1; y++, tr += w) {
            const uint8_t *up = (y > iy0) ? tr - w : z;
            const uint8_t *dn = (y + 1u >= iy0 && y + 1u < iy1) ? tr + w : z;
            for (uint32_t x = ox0; x < ox1; x++) {
                if (tr[x]) continue;
                if (((uint32_t)(up[x] - 1u) < INK_MAX) || ((uint32_t)(dn[x] - 1u) < INK_MAX) ||
                    (x && (uint32_t)(tr[x - 1u] - 1u) < INK_MAX) || (x + 1u < w && (uint32_t)(tr[x + 1u] - 1u) < INK_MAX))
                    tr[x] = OUTLINE;
            }
        }
        return;
    }
    for (uint32_t y = iy0; y < iy1; y++) {
        const uint8_t *tr = t + y * w;
        uint8_t *mr = m + y * w;
        uint32_t win = ox0 ? 0u : ((((uint32_t)(tr[0] - 1u) < INK_MAX) << 1) | ((uint32_t)(tr[1] - 1u) < INK_MAX));
        for (uint32_t x = ox0; x < ox1; x++) {
            uint32_t nxt = (x + 2u < w) && ((uint32_t)(tr[x + 2u] - 1u) < INK_MAX);
            win = ((win << 1) | nxt) & 0x1Fu;
            mr[x] = (uint8_t)(win ? ((win & 0x0Eu) ? 3u : 2u) : 0u);
        }
    }
    uint32_t len = ox1 - ox0;
    for (uint32_t y = oy0; y < oy1; y++) {
        uint8_t *tr = t + y * w + ox0;
        const uint8_t *m0 = (y >= iy0 + 2u && y - 2u < iy1) ? m + (y - 2u) * w + ox0 : z;
        const uint8_t *m1 = (y >= iy0 + 1u && y - 1u < iy1) ? m + (y - 1u) * w + ox0 : z;
        const uint8_t *m2 = (y >= iy0 && y < iy1) ? m + y * w + ox0 : z;
        const uint8_t *m3 = (y + 1u >= iy0 && y + 1u < iy1) ? m + (y + 1u) * w + ox0 : z;
        const uint8_t *m4 = (y + 2u >= iy0 && y + 2u < iy1) ? m + (y + 2u) * w + ox0 : z;
        uint32_t cnt = len;
        __asm__ volatile(
            "1:\n\t"
            "subs %[cnt], %[cnt], #1\n\t"
            "ldrb r0, [%[tr], %[cnt]]\n\t"
            "cmp r0, #0\n\t"
            "bne 2f\n\t"
            "ldrb r0, [%[m0], %[cnt]]\n\t"
            "ldrb r1, [%[m4], %[cnt]]\n\t"
            "orr r0, r0, r1\n\t"
            "and r0, r0, #1\n\t"
            "ldrb r1, [%[m1], %[cnt]]\n\t"
            "orr r0, r0, r1\n\t"
            "ldrb r1, [%[m2], %[cnt]]\n\t"
            "orr r0, r0, r1\n\t"
            "ldrb r1, [%[m3], %[cnt]]\n\t"
            "orrs r0, r0, r1\n\t"
            "movne r0, #16\n\t"
            "strbne r0, [%[tr], %[cnt]]\n\t"
            "2:\n\t"
            "cmp %[cnt], #0\n\t"
            "bne 1b\n\t"
            : [cnt] "+r"(cnt)
            : [tr] "r"(tr), [m0] "r"(m0), [m1] "r"(m1), [m2] "r"(m2), [m3] "r"(m3), [m4] "r"(m4)
            : "r0", "r1", "cc", "memory");
    }
}

/* film label for the LV color mode icon about to be looked up; 0 = stock icon */
static uint32_t osd_label;

static void reset(void) {
    memset(&cache, 0, sizeof cache);
    cache.magic = MAGIC;
    osd_label = 0;
}

static uint8_t *alloc(uint32_t n) {
    if (cache.top + n > ARENA) reset(); /* drops every tile: callers blit before the next lookup */
    uint8_t *p = arena + cache.top;
    cache.top += (n + 3u) & ~3u;
    return p;
}

// split after the space whose blank column run gives the largest, then most balanced, 2 lines; 0 = 1 line
static uint32_t best_split(const char *s, struct box b, uint32_t aw, uint32_t ah, uint32_t ring, uint32_t *px) {
    uint16_t g0[LEN_MAX], g1[LEN_MAX];
    uint32_t ng = 0, nsp = 0, split = 0, best = 0;
    for (const char *p = s; *p; p++) nsp += *p == ' ';
    for (uint32_t x = b.x0; x < b.x1 && nsp && ng < LEN_MAX;) {
        uint32_t e = x;
        while (!cols[e]) e++;
        if (e > x) { g0[ng] = (uint16_t)(x - b.x0); g1[ng++] = (uint16_t)(e - b.x0); }
        x = e + 1;
    }
    /* spaces are the nsp widest runs: drop the narrowest until the counts match */
    while (ng > nsp) {
        uint32_t k = 0;
        for (uint32_t j = 1; j < ng; j++) k = g1[j] - g0[j] < g1[k] - g0[k] ? j : k;
        for (ng--; k < ng; k++) { g0[k] = g0[k + 1]; g1[k] = g1[k + 1]; }
    }
    if (ng != nsp) return 0;
    for (uint32_t j = 0, i = 0; j < ng; j++, i++) {
        while (s[i] != ' ') i++;
        uint32_t w = max(g0[j], b.x1 - b.x0 - g1[j]);
        uint32_t p = min(udiv(PX_REF * aw, w), udiv(PX_REF * ((ah - ring - 1u) >> 1), b.y1 - b.y0));
        if (p > *px || (split && p == *px && w < best)) { *px = p; split = i + 1u; best = w; }
    }
    return split;
}

// uppercase, centred, as large as fits on 1 line or 2 balanced lines
static uint8_t *render(const struct cls *c, const char *src) {
    char s[LEN_MAX];
    uint32_t n = 0, has_sp = 0;
    for (; src[n] && n < LEN_MAX - 1; n++) {
        char ch = src[n];
        if (ch == ' ') has_sp = 1u;
        s[n] = (ch >= 'a' && ch <= 'z') ? (char)(ch - 32) : ch;
    }
    s[n] = 0;

    struct box b = draw(c, s, -1, PX_REF);
    if (!b.y1) return 0;
    uint32_t bold = 1u, r = c->ring;
    uint32_t aw = c->w - 2u * (c->mx + r) - bold, ah = c->h - 2u * (c->my + r);
    uint32_t px = min(udiv(PX_REF * aw, b.x1 - b.x0), udiv(PX_REF * min(ah, c->hmax - 2u * r), b.y1 - b.y0));
    uint32_t split = has_sp ? best_split(s, b, aw, ah, r, &px) : 0u;

    struct box b1, b2 = { 0, 0, 0, 0 };
    uint32_t w1, h1, w2 = 0, h2 = 0, tot;
    uint32_t have_ref = 1u;
    for (px = min(max(px, PX_MIN), PX_MAX);; px--) {
        r = px >= PX_RING2 ? c->ring : 1u;
        bold = (c->flags & BOLD) || (px >= PX_BOLD0 && px < PX_BOLD1);
        aw = c->w - 2u * (c->mx + r) - bold;
        ah = c->h - 2u * (c->my + r);
        if (!split && have_ref && px == PX_REF) {
            b1 = b;
        } else {
            have_ref = 0u;
            b1 = draw(c, s, split ? (int32_t)split - 1 : -1, px);
        }
        w1 = min(b1.x1 - b1.x0, c->w);
        h1 = tot = min(b1.y1 - b1.y0, c->h);
        if (split) {
            for (uint32_t y = 0; y < h1; y++) memcpy(scratch + y * w1, LINE_BUF + (b1.y0 + y) * LB_W + b1.x0, w1);
            b2 = draw(c, s + split, -1, px);
            w2 = min(b2.x1 - b2.x0, c->w);
            h2 = min(b2.y1 - b2.y0, c->h);
            tot += r + 1u + h2;
        }
        if ((max(w1, w2) <= aw && tot <= ah) || px <= PX_MIN) break;
    }

    uint8_t *t = alloc(c->w * c->h);
    memset(t, 0, c->w * c->h);
    int32_t y0_p = ((int32_t)c->h - (int32_t)tot) / 2, x_l = (int32_t)(c->mx + r);
    int32_t y = y0_p;
    int32_t x1_p = (c->flags & LEFT) ? x_l : ((int32_t)(c->w - w1) - (int32_t)bold) / 2;
    int32_t x2_p = split ? ((c->flags & LEFT) ? x_l : ((int32_t)(c->w - w2) - (int32_t)bold) / 2) : x1_p;
    const uint8_t *last = LINE_BUF + (split ? b2.y0 * LB_W + b2.x0 : b1.y0 * LB_W + b1.x0);
    if (split) {
        put(t, c, x1_p, y, scratch, w1, h1, w1);
        y += (int32_t)(h1 + r + 1u);
    }
    put(t, c, x2_p, y, last, split ? w2 : w1, split ? h2 : h1, LB_W);
    int32_t min_x = split && x1_p < x2_p ? x1_p : x2_p;
    int32_t max_x = x1_p + (int32_t)w1;
    if (split && x2_p + (int32_t)w2 > max_x) max_x = x2_p + (int32_t)w2;
    int32_t max_y = y + (int32_t)(split ? h2 : h1);
    struct box ink = {
        min_x > 0 ? (uint32_t)min_x : 0u,
        y0_p > 0 ? (uint32_t)y0_p : 0u,
        max_x > 0 ? min((uint32_t)max_x, c->w) : 0u,
        max_y > 0 ? min((uint32_t)max_y, c->h) : 0u,
    };
    finish_tile(t, c->w, c->h, bold, r, scratch, ink);
    return t;
}

// icon lookup for ids >= ICON_LABEL_L (icon_hook); UI task only, as it shares the stock line buffer
const struct icon *label_icon(uint32_t id) {
    uint32_t k = (id >> 10) - 1u, i = id & 0x3FFu;
    if (k >= N_CLASSES || i >= dp2m_string_count) return 0;
    if (cache.magic != MAGIC) reset();
    const struct cls *c = &classes[k];
    uint8_t *t = cache.tile[k][i];
    if (!t) {
        uint32_t num = REG32(FONT_SCALE), den = REG32(FONT_SCALE + 4u);
        uint32_t d0 = REG32(OSD_DIRTY), d1 = REG32(OSD_DIRTY + 4u);
        stock_font_scale(1, 1);
        t = render(c, dp2m_strings[i]);
        stock_font_scale(num, den);
        REG32(OSD_DIRTY) = d0;
        REG32(OSD_DIRTY + 4u) = d1;
        if (!t) return 0;
        cache.tile[k][i] = t;
        hw_dcache_sync();
    }
    ICON_RESULT->data = t;
    ICON_RESULT->w = c->w;
    ICON_RESULT->h = c->h;
    return ICON_RESULT;
}

// boot init 0xA01689BC: SDRAM may keep decayed .bss across a short power cycle
void labels_boot(void) {
    reset();
    memset((void *)&lv_planes, 0, sizeof lv_planes);
    stock_disp_init();
}
HOOK_CALL(0xA01689BE, labels_boot, 0xA0168D0C);

// films take the Vivid / B&W case of the stock color mode icon switches; osd_icon then swaps in their label
uint32_t osd_map(uint32_t cm) {
    const struct film_info *f = film_of(cm);
    osd_label = f ? ICON_LABEL_OSD + (cm - FILM_MODE0) : 0; /* film labels are the first strings */
    return f ? (f->mono ? CM_BW : CM_VIVID) : cm;
}

const struct icon *osd_icon(uint32_t id) {
    const struct icon *ic = osd_label ? stock_icon(osd_label) : 0;
    osd_label = 0;
    return ic ? ic : stock_icon(id);
}
HOOK_CALL(0xA016BC86, osd_icon, 0xA0042AAC);
HOOK_CALL(0xA004C80A, osd_icon, 0xA0042AAC);

// color mode icon drawers without a film case; the label is dropped if the drawer bails before its lookup
void lv_status_cm(uint32_t cm) {
    stock_lv_status_cm(osd_map(cm));
    osd_label = 0;
}
HOOK_CALL(0xA0048A62, lv_status_cm, 0xA004C7BC);

// playback detailed info (DISP mode 5); cm is the file's, from its info +0x17
void pb_info_cm(uint32_t cm) {
    stock_pb_info_cm(osd_map(cm));
    osd_label = 0;
}
HOOK_CALL(0xA0048016, pb_info_cm, 0xA0047734);
HOOK_CALL(0xA0047774, osd_icon, 0xA0042AAC);
