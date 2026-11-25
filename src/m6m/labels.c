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
    uint8_t *tile[N_CLASSES][1024];
} cache;
static uint8_t arena[ARENA] __attribute__((aligned(4)));
static uint8_t scratch[TILE_MAX]; /* first line of a 2-line label, then the outline mask */
static uint8_t cols[LB_W];

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

// ink box of s[..n] drawn offscreen (rect y >= 480 leaves the OSD untouched); marks inked columns in occ
static struct box draw(const struct cls *c, const char *s, int32_t n, uint32_t px, uint8_t *occ) {
    static const uint16_t offscreen[4] = { 0, 600, LB_W, 100 };
    stock_text(0, s, n, px, c->weight, 0, !(c->flags & BOLD), offscreen, 0x21, 0);
    uint32_t rows = rows_of(px);
    struct box b = { LB_W, 0, 0, 0 };
    for (uint32_t y = 0; y < rows; y++) {
        const uint8_t *r = LINE_BUF + y * LB_W;
        for (uint32_t x = 0; x < LB_W; x += 4) {
            if (!*(const uint32_t *)(const void *)(r + x)) continue;
            for (uint32_t k = x; k < x + 4; k++) {
                if (!r[k]) continue;
                if (!b.y1) b.y0 = y;
                b.x0 = min(b.x0, k);
                b.x1 = max(b.x1, k + 1);
                b.y1 = y + 1;
                if (occ) occ[k] = 1;
            }
        }
    }
    if (!b.y1) b.x0 = 0;
    return b;
}

static void put(uint8_t *t, const struct cls *c, int32_t x, int32_t y, const uint8_t *src, uint32_t w, uint32_t h, uint32_t stride) {
    for (uint32_t j = 0; j < h; j++) {
        uint32_t ty = (uint32_t)(y + (int32_t)j);
        for (uint32_t i = 0; i < w; i++) {
            uint32_t tx = (uint32_t)(x + (int32_t)i);
            if (ty < c->h && tx < c->w && src[j * stride + i]) t[ty * c->w + tx] = src[j * stride + i];
        }
    }
}

// MODE / widget tokens & mid-size text: strokes 1 px heavier
static void embolden(uint8_t *t, uint32_t w, uint32_t h) {
    for (uint32_t y = 0; y < h; y++)
        for (uint32_t x = w - 1; x > 0; x--) {
            uint8_t *p = &t[y * w + x], l = p[-1];
            if (l && (!*p || l < *p)) *p = l;
        }
}

// stock outline shapes: r = 1 plus, r = 2 the 5x5 square without corners; m gets the row dilations
static void outline(uint8_t *t, uint32_t w, uint32_t h, uint32_t r, uint8_t *m) {
    for (uint32_t y = 0; y < h; y++)
        for (uint32_t x = 0; x < w; x++) {
            uint32_t bits = 0;
            for (uint32_t k = 0; k <= 2 * r; k++) {
                uint32_t xx = x + k - r, v = xx < w ? t[y * w + xx] : 0;
                if (v && v <= INK_MAX) bits |= (k == 0 || k == 2 * r) ? 2u : 3u;
            }
            m[y * w + x] = (uint8_t)bits;
        }
    for (uint32_t y = 0; y < h; y++)
        for (uint32_t x = 0; x < w; x++) {
            if (t[y * w + x]) continue;
            for (uint32_t k = 0; k <= 2 * r; k++) {
                uint32_t yy = y + k - r;
                if (yy < h && (m[yy * w + x] & ((k == 0 || k == 2 * r) ? 1u : 2u))) {
                    t[y * w + x] = OUTLINE;
                    break;
                }
            }
        }
}

static void reset(void) {
    memset(&cache, 0, sizeof cache);
    cache.magic = MAGIC;
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
    uint32_t n = 0;
    for (; src[n] && n < LEN_MAX - 1; n++) s[n] = (src[n] >= 'a' && src[n] <= 'z') ? (char)(src[n] - 32) : src[n];
    s[n] = 0;

    memset(cols, 0, sizeof cols);
    struct box b = draw(c, s, -1, PX_REF, cols);
    if (!b.y1) return 0;
    uint32_t bold = 1u, r = c->ring;
    uint32_t aw = c->w - 2u * (c->mx + r) - bold, ah = c->h - 2u * (c->my + r);
    uint32_t px = min(udiv(PX_REF * aw, b.x1 - b.x0), udiv(PX_REF * min(ah, c->hmax - 2u * r), b.y1 - b.y0));
    uint32_t split = best_split(s, b, aw, ah, r, &px);

    struct box b1, b2 = { 0, 0, 0, 0 };
    uint32_t w1, h1, w2 = 0, h2 = 0, tot;
    for (px = min(max(px, PX_MIN), PX_MAX);; px--) {
        r = px >= PX_RING2 ? c->ring : 1u;
        bold = (c->flags & BOLD) || (px >= PX_BOLD0 && px < PX_BOLD1);
        aw = c->w - 2u * (c->mx + r) - bold;
        ah = c->h - 2u * (c->my + r);
        b1 = draw(c, s, split ? (int32_t)split - 1 : -1, px, 0);
        w1 = min(b1.x1 - b1.x0, c->w);
        h1 = tot = min(b1.y1 - b1.y0, c->h);
        if (split) {
            for (uint32_t y = 0; y < h1; y++) memcpy(scratch + y * w1, LINE_BUF + (b1.y0 + y) * LB_W + b1.x0, w1);
            b2 = draw(c, s + split, -1, px, 0);
            w2 = min(b2.x1 - b2.x0, c->w);
            h2 = min(b2.y1 - b2.y0, c->h);
            tot += r + 1u + h2;
        }
        if ((max(w1, w2) <= aw && tot <= ah) || px <= PX_MIN) break;
    }

    uint8_t *t = alloc(c->w * c->h);
    memset(t, 0, c->w * c->h);
    int32_t y = ((int32_t)c->h - (int32_t)tot) / 2, x = (int32_t)(c->mx + r);
    const uint8_t *last = LINE_BUF + (split ? b2.y0 * LB_W + b2.x0 : b1.y0 * LB_W + b1.x0);
    if (split) {
        put(t, c, (c->flags & LEFT) ? x : ((int32_t)(c->w - w1) - (int32_t)bold) / 2, y, scratch, w1, h1, w1);
        y += (int32_t)(h1 + r + 1u);
    }
    put(t, c, (c->flags & LEFT) ? x : ((int32_t)(c->w - (split ? w2 : w1)) - (int32_t)bold) / 2, y, last, split ? w2 : w1, split ? h2 : h1, LB_W);
    if (bold) embolden(t, c->w, c->h);
    outline(t, c->w, c->h, r, scratch);
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
    }
    ICON_RESULT->data = t;
    ICON_RESULT->w = c->w;
    ICON_RESULT->h = c->h;
    return ICON_RESULT;
}

// boot init 0xA01689BC: SDRAM may keep a decayed cache across a short power cycle
void labels_boot(void) {
    reset();
    stock_disp_init();
}
HOOK_CALL(0xA01689BE, labels_boot, 0xA0168D0C);

/* film label for the LV color mode icon about to be looked up; 0 = stock icon */
static uint32_t osd_label;

// films take the Vivid / B&W case of the stock color mode icon switches; osd_icon then swaps in their label
uint32_t osd_map(uint32_t cm) {
    uint32_t f = cm - FILM_MODE0;
    osd_label = f < dp2m_film_count ? ICON_LABEL_OSD + f : 0; /* film labels are the first strings */
    return osd_label ? (dp2m_films[f].mono ? CM_BW : CM_VIVID) : cm;
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
