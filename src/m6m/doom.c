/* Bare-metal ARM946E-S Big-Endian DOOM + SD Applet Loader for Sigma DP2 Merrill */
#include "doom.h"
#include "dp2m_app.h"

extern uint8_t __bss_end[];

#define DOOM_MAGIC 0x444F4F4Du /* "DOOM" */

/* Runtime RAM buffers in payload .bss (0xA1C00000..0xA1E00000) */
struct d_state doom_st;
uint8_t doom_fb[D_W * D_H] __attribute__((aligned(4)));
uint8_t doom_tex[TEX_COUNT][64 * 64] __attribute__((aligned(4)));
uint8_t doom_spr[7][32 * 32] __attribute__((aligned(4)));
uint8_t doom_cmap[32][256] __attribute__((aligned(4)));
uint32_t doom_pal32[3][256] __attribute__((aligned(4)));
int16_t doom_sin[1024] __attribute__((aligned(4)));

/* E1M1 Hangar: 13 strictly convex CCW sectors, 66 linedefs */
const struct d_sector_def doom_map_secs[DOOM_SECS] = {
    /* 0  Start Room            */ {   0, 128, FLAT_BLUE,   FLAT_CEIL1, 224, 0,           0, 6 },
    /* 1  Main Foyer            */ {   0, 160, FLAT_FLOOR1, FLAT_CEIL1, 192, 0,           6, 6 },
    /* 2  Courtyard Overlook    */ {   8, 144, TEX_STEP,    FLAT_CEIL1, 224, 0,          12, 4 },
    /* 3  South Supply Alcove   */ {   0, 112, FLAT_BRIDGE, FLAT_CEIL1, 144, 0,          16, 4 },
    /* 4  Stair Step 1          */ {  12, 160, TEX_STEP,    FLAT_CEIL1, 192, 0,          20, 4 },
    /* 5  Stair Step 2          */ {  24, 168, TEX_STEP,    FLAT_CEIL1, 200, 0,          24, 4 },
    /* 6  Computer Room         */ {  24, 176, FLAT_FLOOR1, FLAT_CEIL1, 216, SEC_BLINK,  28, 8 },
    /* 7  South Nukage Pit      */ { -24, 184, FLAT_NUKAGE, FLAT_CEIL1, 176, SEC_NUKAGE, 36, 4 },
    /* 8  Zig-Zag Bridge        */ {  24, 184, FLAT_BRIDGE, FLAT_CEIL1, 204, 0,          40, 4 },
    /* 9  North Nukage Pit      */ { -24, 184, FLAT_NUKAGE, FLAT_CEIL1, 176, SEC_NUKAGE, 44, 4 },
    /* 10 Sliding Tech Door     */ {  24, 120, TEX_STEP,    FLAT_CEIL1, 180, SEC_DOOR,   48, 4 },
    /* 11 Exit Switch Room      */ {  32, 152, FLAT_BLUE,   FLAT_CEIL1, 224, 0,          52, 6 },
    /* 12 Outdoor Courtyard     */ {   0, 240, FLAT_GRASS,  TEX_SKY,    248, 0,          58, 8 },
};

const struct d_line doom_map_lines[DOOM_LINES] = {
    /* S0 (0..5) */
    {    0,   64,   64,    0, -1, TEX_STARTAN, 0 },
    {   64,    0,  256,    0, -1, TEX_STARTAN, 0 },
    {  256,    0,  256,  256,  1, TEX_TECH,    0 },
    {  256,  256,   64,  256, -1, TEX_STARTAN, 0 },
    {   64,  256,    0,  192, -1, TEX_TECH,    0 },
    {    0,  192,    0,   64, -1, TEX_TECH,    0 },
    /* S1 (6..11) */
    {  256,    0,  480,    0,  3, TEX_STARTAN, 0 },
    {  480,    0,  544,   64, -1, TEX_TECH,    0 },
    {  544,   64,  544,  192,  4, TEX_STARTAN, 0 },
    {  544,  192,  480,  256, -1, TEX_TECH,    0 },
    {  480,  256,  256,  256,  2, TEX_BRICK,   0 },
    {  256,  256,  256,    0,  0, TEX_TECH,    0 },
    /* S2 (12..15) */
    {  256,  256,  480,  256,  1, TEX_BRICK,   0 },
    {  480,  256,  480,  320, -1, TEX_BRICK,   0 },
    {  480,  320,  256,  320, 12, TEX_BRICK,   0 },
    {  256,  320,  256,  256, -1, TEX_BRICK,   0 },
    /* S3 (16..19) */
    {  288, -128,  448, -128, -1, TEX_TECH,    0 },
    {  448, -128,  480,    0, -1, TEX_STARTAN, 0 },
    {  480,    0,  256,    0,  1, TEX_STARTAN, 0 },
    {  256,    0,  288, -128, -1, TEX_STARTAN, 0 },
    /* S4 (20..23) */
    {  544,   64,  592,   64, -1, TEX_STARTAN, 0 },
    {  592,   64,  592,  192,  5, TEX_STARTAN, 0 },
    {  592,  192,  544,  192, -1, TEX_STARTAN, 0 },
    {  544,  192,  544,   64,  1, TEX_STARTAN, 0 },
    /* S5 (24..27) */
    {  592,   64,  640,   64, -1, TEX_STARTAN, 0 },
    {  640,   64,  640,  192,  6, TEX_TECH,    0 },
    {  640,  192,  592,  192, -1, TEX_STARTAN, 0 },
    {  592,  192,  592,   64,  4, TEX_STARTAN, 0 },
    /* S6 (28..35) */
    {  640,   64,  704,    0, -1, TEX_TECH,    0 },
    {  704,    0,  864,    0, -1, TEX_TECH,    0 },
    {  864,    0,  864,   64,  7, TEX_BRICK,   0 },
    {  864,   64,  864,  192,  8, TEX_STARTAN, 0 },
    {  864,  192,  864,  256,  9, TEX_BRICK,   0 },
    {  864,  256,  704,  256, -1, TEX_TECH,    0 },
    {  704,  256,  640,  192, -1, TEX_TECH,    0 },
    {  640,  192,  640,   64,  5, TEX_TECH,    0 },
    /* S7 (36..39) */
    {  864,    0, 1120,    0, -1, TEX_BRICK,   0 },
    { 1120,    0, 1120,  112, -1, TEX_BRICK,   0 },
    { 1120,  112,  864,   64,  8, TEX_STEP,    0 },
    {  864,   64,  864,    0,  6, TEX_BRICK,   0 },
    /* S8 (40..43) */
    {  864,   64, 1120,  112,  7, TEX_STEP,    0 },
    { 1120,  112, 1120,  208, 10, TEX_DOOR,    LINE_DOOR },
    { 1120,  208,  864,  192,  9, TEX_STEP,    0 },
    {  864,  192,  864,   64,  6, TEX_STARTAN, 0 },
    /* S9 (44..47) */
    {  864,  192, 1120,  208,  8, TEX_STEP,    0 },
    { 1120,  208, 1120,  320, -1, TEX_BRICK,   0 },
    { 1120,  320,  864,  256, -1, TEX_BRICK,   0 },
    {  864,  256,  864,  192,  6, TEX_BRICK,   0 },
    /* S10 (48..51) */
    { 1120,  112, 1152,  112, -1, TEX_STEP,    0 },
    { 1152,  112, 1152,  208, 11, TEX_DOOR,    LINE_DOOR },
    { 1152,  208, 1120,  208, -1, TEX_STEP,    0 },
    { 1120,  208, 1120,  112,  8, TEX_DOOR,    LINE_DOOR },
    /* S11 (52..57) */
    { 1152,  112, 1216,   64, -1, TEX_TECH,    0 },
    { 1216,   64, 1344,   64, -1, TEX_STARTAN, 0 },
    { 1344,   64, 1344,  256, -1, TEX_EXIT,    LINE_EXIT },
    { 1344,  256, 1216,  256, -1, TEX_STARTAN, 0 },
    { 1216,  256, 1152,  208, -1, TEX_TECH,    0 },
    { 1152,  208, 1152,  112, 10, TEX_DOOR,    LINE_DOOR },
    /* S12 (58..65) */
    {  192,  320,  256,  320, -1, TEX_BRICK,   0 },
    {  256,  320,  480,  320,  2, TEX_BRICK,   0 },
    {  480,  320,  544,  320, -1, TEX_BRICK,   0 },
    {  544,  320,  544,  512, -1, TEX_BRICK,   0 },
    {  544,  512,  448,  576, -1, TEX_BRICK,   0 },
    {  448,  576,  288,  576, -1, TEX_BRICK,   0 },
    {  288,  576,  192,  512, -1, TEX_BRICK,   0 },
    {  192,  512,  192,  320, -1, TEX_BRICK,   0 },
};

static const struct { int16_t x, y; uint8_t kind, sec, hp; } init_things[9] = {
    {  400, 128, TH_ZOMBIE,  1,  30 },
    {  430, 420, TH_IMP,    12,  60 },
    {  368, 464, TH_ARMOR,  12,   0 },
    {  336, -64, TH_MEDIKIT, 3,   0 },
    {  400, -64, TH_AMMO,    3,   0 },
    {  768, 176, TH_IMP,     6,  60 },
    {  768,  80, TH_ZOMBIE,  6,  30 },
    {  992, 152, TH_DEMON,   8, 100 },
    { 1264, 160, TH_IMP,    11,  60 },
};

/* 4x5 digit + '%' bitmaps (each nibble = 4px row, 5 rows = 20 bits) */
static const uint32_t hud_digits[11] = {
    0xF999F, 0x26227, 0xF1F8F, 0xF1F1F, 0x99F11,
    0xF8F1F, 0xF8F9F, 0xF1222, 0xF9F9F, 0xF9F1F, 0x92490,
};

/* 3x4 micro uppercase font (4 rows x 3 bits packed in nibbles) */
static const uint16_t micro_font[26] = {
    0x7575, 0x6567, 0x7447, 0x6556, 0x7647, 0x7644, 0x7457, 0x5755, 0x7227,
    0x1157, 0x5655, 0x4447, 0x5755, 0x6555, 0x7557, 0x7574, 0x7563, 0x7565,
    0x3616, 0x7222, 0x5557, 0x5552, 0x5575, 0x5255, 0x5222, 0x7127,
};

/* 14 palette ramps x 16 shades (0..223) */
static const uint8_t ramp_rgb[14][3] = {
    { 240, 240, 240 }, { 196, 168, 128 }, { 148, 124,  88 }, { 164, 108,  68 },
    {  96,  72,  52 }, {  64, 108, 224 }, {  64, 240,  48 }, { 236,  44,  32 },
    { 252, 200,  48 }, {  56, 212, 224 }, { 228, 152, 120 }, {  92, 128,  72 },
    {  84, 104, 136 }, { 208, 132,  84 },
};

/* ---------- Cold init & procedural asset generation (DOOM_E) ---------- */

static uint32_t doom_rand(void) {
    doom_st.rng = doom_st.rng * 1664525u + 1013904223u;
    return doom_st.rng >> 16;
}

__attribute__((section(".doom.init")))
static uint32_t rgb_pair(int32_t r, int32_t g, int32_t b) {
    uint32_t c = ((uint32_t)(d_clamp(r, 0, 255) >> 4) << 12)
               | ((uint32_t)(d_clamp(g, 0, 255) >> 4) << 8)
               | ((uint32_t)(d_clamp(b, 0, 255) >> 4) << 4) | 0xFu;
    return (c << 16) | c;
}

__attribute__((section(".doom.init")))
static void init_trig_and_pal(void) {
    /* Quadratic-blended parabolic quarter-wave Q14 sine (max err < 0.2%) */
    for (uint32_t i = 0; i < 256u; i++) {
        uint32_t p = i * (512u - i), p2 = (p * p) >> 16;
        int32_t s = d_clamp((int32_t)((p * 198u + p2 * 58u) >> 10), 0, 16384);
        doom_sin[i] = doom_sin[512u - i] = (int16_t)s;
        doom_sin[512u + i] = doom_sin[(1024u - i) & 1023u] = (int16_t)(-s);
    }
    doom_sin[256] = 16384;
    doom_sin[768] = -16384;

    for (uint32_t i = 0; i < 256u; i++) {
        int32_t r = 0, g = 0, b = 0;
        if (i < 224u) {
            uint32_t ramp = i >> 4, lvl = i & 15u;
            int32_t k = (int32_t)(ramp >= 12u ? 128u + lvl * 8u : 256u - lvl * 14u);
            r = (ramp_rgb[ramp][0] * k) >> 8;
            g = (ramp_rgb[ramp][1] * k) >> 8;
            b = (ramp_rgb[ramp][2] * k) >> 8;
        }
        doom_pal32[0][i] = rgb_pair(r, g, b);
        doom_pal32[1][i] = rgb_pair(r + 120, g >> 1, b >> 1);
        doom_pal32[2][i] = rgb_pair(r + 80, g + 64, b >> 1);

        for (uint32_t sh = 0; sh < 32u; sh++) {
            uint32_t idx = (i & 15u) + (sh >> 1);
            doom_cmap[sh][i] = (uint8_t)(i < 192u ? ((i & 0xF0u) | (idx > 15u ? 15u : idx)) : i);
        }
    }
}

/* Walls are column-major tex[u*64 + v], flats are row-major flat[v*64 + u] */
__attribute__((section(".doom.init")))
static void gen_textures(void) {
    for (uint32_t u = 0; u < 64u; u++) {
        for (uint32_t v = 0; v < 64u; v++) {
            uint32_t h = ((u * 73u) ^ (v * 151u) ^ ((u + v) * 37u)) & 7u;
            uint32_t h3 = h & 3u, widx = (u << 6) | v, fidx = (v << 6) | u;
            uint32_t c;

            /* 0: TEX_STARTAN */
            c = 18u + h3;
            if (v == 0 || v == 31 || v == 32 || v == 63) c = 28u;
            else if (v == 1 || v == 33) c = 16u;
            else if ((u & 15u) == 0u) c = 27u;
            else if ((u & 15u) == 1u) c = 17u;
            else if ((v == 4 || v == 28 || v == 36 || v == 60) && ((u & 15u) == 3u || (u & 15u) == 12u)) c = 12u;
            else if (v >= 54u) c = 36u + h3;
            doom_tex[TEX_STARTAN][widx] = (uint8_t)c;

            /* 1: TEX_TECH */
            c = 6u + h3;
            if ((u & 31u) == 0u || (v & 31u) == 0u) c = 14u;
            else if ((u & 31u) == 1u || (v & 31u) == 1u) c = 3u;
            else if (u >= 6u && u <= 26u && v >= 8u && v <= 24u) {
                c = (u == 6u || u == 26u || v == 8u || v == 24u) ? 13u
                  : ((v & 3u) == 0u) ? 148u : (144u + ((u ^ v) & 3u));
            } else if (u >= 38u && u <= 58u && v >= 10u && v <= 22u) {
                c = ((u & 7u) < 4u && (v & 3u) < 2u) ? (((u >> 3) & 1u) ? 96u : 112u) : 13u;
            } else if (v >= 38u && v <= 56u && (v & 3u) == 0u && u >= 6u && u <= 58u) {
                c = 14u;
            }
            doom_tex[TEX_TECH][widx] = (uint8_t)c;

            /* 2: TEX_BRICK */
            uint32_t row = v >> 4, bu = (u + (row & 1u) * 16u) & 31u, bv = v & 15u;
            c = (bv == 0u || bu == 0u) ? 46u
              : (bv == 1u || bu == 1u) ? 33u
              : 36u + ((row ^ (bu >> 4)) & 3u) + h3;
            doom_tex[TEX_BRICK][widx] = (uint8_t)c;

            /* 3: TEX_DOOR */
            c = 4u + (h & 1u);
            if (v >= 56u) c = (((u + v) >> 3) & 1u) ? 130u : 14u;
            else if (u < 6u || u >= 58u) c = 10u + (v & 1u);
            else if (u == 31u || u == 32u) c = 14u;
            else if ((v & 7u) == 0u) c = 11u;
            else if (v >= 22u && v <= 42u && u >= 24u && u <= 40u)
                c = (u == 24u || u == 40u || v == 22u || v == 42u) ? 13u : 114u;
            doom_tex[TEX_DOOR][widx] = (uint8_t)c;

            /* 4: TEX_EXIT */
            c = 8u + h3;
            if (u >= 10u && u <= 54u && v >= 10u && v <= 54u) {
                if (u == 10u || u == 54u || v == 10u || v == 54u) c = 130u;
                else if (v >= 16u && v <= 26u) c = ((u & 3u) != 0u) ? 112u : 120u;
                else if (u >= 22u && u <= 42u && v >= 32u && v <= 48u) c = (v < 40u) ? 128u : 114u;
                else c = 12u;
            }
            doom_tex[TEX_EXIT][widx] = (uint8_t)c;

            /* 5: TEX_STEP */
            doom_tex[TEX_STEP][widx] = (uint8_t)(v < 4u ? 130u : (v & 7u) == 0u ? 14u : 8u + ((u >> 2) & 3u));

            /* 6..11: flats */
            doom_tex[FLAT_BLUE][fidx] = (uint8_t)(82u + (((u & 7u) == 0u || (v & 7u) == 0u) ? 6u : ((u ^ v) & 3u)));
            doom_tex[FLAT_FLOOR1][fidx] = (uint8_t)((u & 31u) == 0u || (v & 31u) == 0u ? 14u
                                                  : (u & 31u) == 1u || (v & 31u) == 1u ? 6u : 9u + (h & 1u));
            c = 10u + (h & 1u);
            if (u >= 20u && u <= 44u && v >= 20u && v <= 44u)
                c = (u == 20u || u == 44u || v == 20u || v == 44u) ? 13u : 0u;
            else if ((u & 15u) == 0u || (v & 15u) == 0u) c = 14u;
            doom_tex[FLAT_CEIL1][fidx] = (uint8_t)c;

            int32_t s1 = d_sin((u << 4) + (v << 3)), s2 = d_sin((v << 4) - (u << 3));
            doom_tex[FLAT_NUKAGE][fidx] = (uint8_t)(96u + ((uint32_t)((s1 + s2 + 32768) >> 13) & 7u));
            doom_tex[FLAT_GRASS][fidx] = (uint8_t)(h3 == 0u ? 178u : 36u + h3);
            doom_tex[FLAT_BRIDGE][fidx] = (uint8_t)(u == 0u || u == 63u ? 132u
                                                  : ((u & 7u) < 2u || (v & 7u) < 2u) ? 6u : 13u);
        }
    }
}

/* 32x32 column-major sprites (0 = transparent) */
__attribute__((section(".doom.init")))
static void gen_sprites(void) {
    memset(doom_spr, 0, sizeof doom_spr);
    for (int32_t x = 0; x < 32; x++) {
        for (int32_t y = 0; y < 32; y++) {
            int32_t dx = x - 16, dy = y - 16, adx = d_abs(dx), r2 = dx * dx + dy * dy;
            uint32_t idx = (uint32_t)((x << 5) | y);
            uint8_t c, xy3 = (uint8_t)((x ^ y) & 3);

            /* 0: TH_ZOMBIE */
            c = 0;
            if (y >= 3 && y <= 9 && adx <= 3) c = (y <= 5) ? 100u : 164u;
            if (y == 6 && (dx == -1 || dx == 2)) c = 112u;
            if (y >= 10 && y <= 21 && adx <= 6) c = 20u + xy3;
            if (y >= 13 && y <= 16 && dx >= 3 && dx <= 10) c = 8u;
            if (y >= 22 && adx >= 1 && adx <= 5) c = 38u;
            doom_spr[0][idx] = c;

            /* 1: TH_IMP */
            c = 0;
            if (y >= 2 && y <= 9 && adx <= 4) c = 52u + xy3;
            if (y <= 4 && adx == 4) c = 18u;
            if (y == 6 && adx == 2) c = 128u;
            if (y >= 10 && y <= 22 && adx <= 7) c = 54u + (uint8_t)((y >> 1) & 3);
            if (y >= 11 && y <= 16 && adx >= 7 && adx <= 11) c = 114u;
            if (y >= 23 && adx >= 2 && adx <= 6) c = 56u;
            doom_spr[1][idx] = c;

            /* 2: TH_DEMON */
            c = 0;
            if (y >= 4 && y <= 18 && adx <= 10) c = 162u + xy3;
            if (y >= 2 && y <= 6 && (adx == 8 || adx == 9)) c = 2u;
            if (y == 9 && adx == 4) c = 128u;
            if (y >= 13 && y <= 16 && adx <= 5) c = (x & 1) ? 2u : 114u;
            if (y >= 19 && adx >= 3 && adx <= 9) c = 166u;
            doom_spr[2][idx] = c;

            /* 3: TH_MEDIKIT */
            c = 0;
            if (x >= 5 && x <= 26 && y >= 10 && y <= 28) {
                c = (x == 5 || x == 26 || y == 10 || y == 28) ? 10u : 2u;
                if ((adx <= 2 && y >= 14 && y <= 24) || (adx <= 6 && y >= 17 && y <= 21)) c = 112u;
            }
            doom_spr[3][idx] = c;

            /* 4: TH_AMMO */
            c = 0;
            if (x >= 6 && x <= 25 && y >= 12 && y <= 29) {
                c = (x == 6 || x == 25 || y == 12 || y == 29) ? 44u : 178u;
                if (y >= 16 && y <= 20 && x >= 9 && x <= 22) c = 128u;
            }
            doom_spr[4][idx] = c;

            /* 5: TH_ARMOR */
            c = 0;
            if (y >= 8 && y <= 27 && adx <= 8 && !(y <= 12 && adx <= 3))
                c = (adx == 8 || y == 27) ? 106u : (98u + (uint8_t)((y >> 2) & 3));
            doom_spr[5][idx] = c;

            /* 6: TH_FIREBALL */
            doom_spr[6][idx] = r2 < 24 ? 128u : r2 < 64 ? 112u : (r2 < 100 && ((x ^ y) & 1)) ? 116u : 0u;
        }
    }
}

__attribute__((section(".doom.init")))
static void doom_reset_level(void) {
    uint32_t rng = doom_st.rng;
    memset(&doom_st, 0, sizeof doom_st);
    doom_st.magic = DOOM_MAGIC;
    doom_st.rng = rng;
    doom_st.px = 88 << 16;
    doom_st.py = 128 << 16;
    doom_st.view_z = 41;
    doom_st.health = 100;
    doom_st.ammo = 50;
    doom_st.face_expr = 1;
    doom_st.face_timer = 12;

    for (uint32_t i = 0; i < DOOM_SECS; i++) {
        const struct d_sector_def *sd = &doom_map_secs[i];
        doom_st.secs[i].ceil_h = (sd->flags & SEC_DOOR) ? sd->floor_h : sd->ceil_h;
    }
    for (uint32_t i = 0; i < 9u; i++) {
        struct d_thing *th = &doom_st.things[i];
        th->x = (int32_t)init_things[i].x << 16;
        th->y = (int32_t)init_things[i].y << 16;
        th->kind = init_things[i].kind;
        th->sector = init_things[i].sec;
        th->hp = init_things[i].hp;
        th->timer = (uint8_t)(10u + i * 3u);
    }
}

__attribute__((section(".doom.init")))
static void doom_ensure_init(void) {
    if (doom_st.magic == DOOM_MAGIC) return;
    doom_st.rng = 0x19931210u;
    init_trig_and_pal();
    gen_textures();
    gen_sprites();
    doom_reset_level();
}

/* ---------- Geometry, Collision & Gameplay (DOOM_B) ---------- */

static int sector_passable(int32_t back, int32_t cur_flr) {
    if (back < 0) return 0;
    int32_t flr = doom_map_secs[back].floor_h;
    return doom_st.secs[back].ceil_h - flr >= 48 && flr <= cur_flr + 24;
}

static int point_in_sector(uint32_t sec_id, int32_t x, int32_t y, int32_t pad) {
    const struct d_sector_def *s = &doom_map_secs[sec_id];
    const struct d_line *lines = &doom_map_lines[s->first_line];
    int32_t mx = x >> 12, my = y >> 12;
    for (uint32_t i = 0; i < s->num_lines; i++) {
        int32_t cross = d_edge_cross_q4(&lines[i], mx, my);
        if (pad > 0) {
            int32_t len = d_oct_dist(lines[i].x2 - lines[i].x1, lines[i].y2 - lines[i].y1);
            if (cross < pad * (len << 4)) return 0;
        } else if (cross < 0) {
            return 0;
        }
    }
    return 1;
}

static int try_move_player(int32_t nx, int32_t ny) {
    const struct d_sector_def *s = &doom_map_secs[doom_st.sector];
    const struct d_line *lines = &doom_map_lines[s->first_line];
    int32_t cur_flr = s->floor_h;

    if (point_in_sector(doom_st.sector, nx, ny, 0)) {
        int32_t mx = nx >> 12, my = ny >> 12;
        for (uint32_t i = 0; i < s->num_lines; i++) {
            if (sector_passable(lines[i].back, cur_flr)) continue;
            int32_t len = d_oct_dist(lines[i].x2 - lines[i].x1, lines[i].y2 - lines[i].y1);
            if (d_edge_cross_q4(&lines[i], mx, my) < 14 * (len << 4)) return 0;
        }
        doom_st.px = nx;
        doom_st.py = ny;
        return 1;
    }

    for (uint32_t i = 0; i < s->num_lines; i++) {
        int32_t b = lines[i].back;
        if (!sector_passable(b, cur_flr)) continue;
        if (point_in_sector((uint32_t)b, nx, ny, 0)) {
            doom_st.px = nx;
            doom_st.py = ny;
            doom_st.sector = (uint8_t)b;
            return 1;
        }
    }
    return 0;
}

static void move_player(int32_t dx, int32_t dy) {
    if (!try_move_player(doom_st.px + dx, doom_st.py + dy))
        if (!try_move_player(doom_st.px + dx, doom_st.py))
            try_move_player(doom_st.px, doom_st.py + dy);
}

__attribute__((section(".doom.extra")))
static void doom_use(void) {
    const struct d_sector_def *s = &doom_map_secs[doom_st.sector];
    const struct d_line *lines = &doom_map_lines[s->first_line];
    int32_t px = doom_st.px >> 16, py = doom_st.py >> 16;

    for (uint32_t i = 0; i < s->num_lines; i++) {
        int32_t mx = ((int32_t)lines[i].x1 + lines[i].x2) >> 1;
        int32_t my = ((int32_t)lines[i].y1 + lines[i].y2) >> 1;
        if (d_abs(mx - px) + d_abs(my - py) > 140) continue;

        if (lines[i].flags & LINE_EXIT) {
            doom_st.won = 1;
            doom_st.bonus_tics = 10;
            doom_st.face_expr = 3;
            doom_st.face_timer = 20;
            return;
        }
        if ((lines[i].flags & LINE_DOOR) && lines[i].back >= 0) {
            struct d_sector_rt *ds = &doom_st.secs[lines[i].back];
            if (ds->door_state == 0 || ds->door_state == 3) ds->door_state = 1;
        }
    }
    if ((doom_st.sector == 8 || doom_st.sector == 11) && doom_st.secs[10].door_state == 0)
        doom_st.secs[10].door_state = 1;
}

__attribute__((section(".doom.extra")))
static void doom_fire(void) {
    static const struct { uint8_t cost, dmg, cone, recoil; } wpn[3] = {
        { 1, 18, 48, 5 }, { 2, 45, 80, 10 }, { 1, 22, 48, 5 },
    };
    if (doom_st.ammo <= 0 || doom_st.health <= 0) return;
    uint32_t w = doom_st.weapon < 3u ? doom_st.weapon : 0u;
    doom_st.ammo = (int16_t)(doom_st.ammo > wpn[w].cost ? doom_st.ammo - wpn[w].cost : 0);
    doom_st.flash_tics = 3;
    doom_st.recoil = wpn[w].recoil;

    int32_t cos_a = d_cos(doom_st.angle), sin_a = d_sin(doom_st.angle);
    int32_t best = -1, best_tz = 0x7FFFFFFF;
    for (int32_t i = 0; i < MAX_THINGS; i++) {
        struct d_thing *th = &doom_st.things[i];
        if (th->kind == TH_NONE || th->kind > TH_DEMON || th->state == 3) continue;
        int32_t dx = (th->x - doom_st.px) >> 12, dy = (th->y - doom_st.py) >> 12;
        int32_t tz = (dx * cos_a + dy * sin_a) >> 14;
        int32_t tx = (dx * sin_a - dy * cos_a) >> 14;
        if (tz < 32 || tz > 9000) continue;
        if (d_abs(tx) * 256 < tz * wpn[w].cone && tz < best_tz) {
            best = i;
            best_tz = tz;
        }
    }

    if (best >= 0) {
        struct d_thing *th = &doom_st.things[best];
        th->hp -= wpn[w].dmg;
        if (th->hp <= 0) {
            th->hp = 0;
            th->state = 3;
            doom_st.kills++;
            doom_st.face_expr = 3;
            doom_st.face_timer = 14;
        } else {
            th->state = 2;
            th->timer = 4;
        }
    }
}

static void hurt_player(int32_t dmg) {
    if (doom_st.health <= 0) return;
    if (doom_st.armor > 0) {
        int32_t absorb = dmg >> 1;
        if (absorb > doom_st.armor) absorb = doom_st.armor;
        doom_st.armor -= (int16_t)absorb;
        dmg -= absorb;
    }
    doom_st.health = (int16_t)(doom_st.health > dmg ? doom_st.health - dmg : 0);
    doom_st.pain_tics = 5;
    doom_st.face_expr = 4;
    doom_st.face_timer = 8;
}

static void doom_step_sim(void) {
    doom_st.tic++;

    if (doom_st.pvx || doom_st.pvy) {
        move_player(doom_st.pvx, doom_st.pvy);
        doom_st.pvx >>= 1;
        doom_st.pvy >>= 1;
    }
    int32_t dz = (doom_map_secs[doom_st.sector].floor_h + 41) - doom_st.view_z;
    if (dz) doom_st.view_z += (dz + (dz > 0 ? 1 : -1)) >> 1;

    if (doom_st.flash_tics) doom_st.flash_tics--;
    if (doom_st.recoil) doom_st.recoil = (uint8_t)(doom_st.recoil > 2 ? doom_st.recoil - 2 : 0);
    if (doom_st.pain_tics) doom_st.pain_tics--;
    if (doom_st.bonus_tics) doom_st.bonus_tics--;

    if (doom_st.face_timer) {
        doom_st.face_timer--;
    } else {
        uint32_t r = doom_rand() & 3u;
        doom_st.face_expr = (uint8_t)(r == 3u ? 1u : r);
        doom_st.face_timer = (uint8_t)(10u + (doom_rand() & 15u));
    }

    for (uint32_t i = 0; i < DOOM_SECS; i++) {
        const struct d_sector_def *sd = &doom_map_secs[i];
        if (!(sd->flags & SEC_DOOR)) continue;
        struct d_sector_rt *s = &doom_st.secs[i];
        if (s->door_state == 1) {
            s->ceil_h += 8;
            if (s->ceil_h >= sd->ceil_h) { s->ceil_h = sd->ceil_h; s->door_state = 2; s->door_timer = 90; }
        } else if (s->door_state == 2) {
            if (s->door_timer) s->door_timer--;
            else if (doom_st.sector != i) s->door_state = 3;
        } else if (s->door_state == 3) {
            if (doom_st.sector == i) { s->door_state = 1; continue; }
            s->ceil_h -= 6;
            if (s->ceil_h <= sd->floor_h) { s->ceil_h = sd->floor_h; s->door_state = 0; }
        }
    }

    if ((doom_map_secs[doom_st.sector].flags & SEC_NUKAGE) && (doom_st.tic & 15u) == 0u)
        hurt_player(5);

    for (int32_t i = 0; i < MAX_THINGS; i++) {
        struct d_thing *th = &doom_st.things[i];
        if (th->kind == TH_NONE) continue;

        int32_t dx = (doom_st.px - th->x) >> 16, dy = (doom_st.py - th->y) >> 16;
        int32_t dist = d_oct_dist(dx, dy);

        if (th->kind >= TH_MEDIKIT && th->kind <= TH_ARMOR) {
            if (dist < 32) {
                if (th->kind == TH_MEDIKIT && doom_st.health < 100)
                    doom_st.health = (int16_t)(doom_st.health + 25 > 100 ? 100 : doom_st.health + 25);
                else if (th->kind == TH_AMMO) doom_st.ammo += 24;
                else if (th->kind == TH_ARMOR) doom_st.armor = 100;
                else continue;
                th->kind = TH_NONE;
                doom_st.bonus_tics = 5;
                doom_st.face_expr = 3;
                doom_st.face_timer = 12;
            }
            continue;
        }

        if (th->kind == TH_FIREBALL) {
            th->x += th->vx;
            th->y += th->vy;
            if (dist < 22) { hurt_player(8); th->kind = TH_NONE; continue; }
            if (!point_in_sector(th->sector, th->x, th->y, 0)) {
                const struct d_sector_def *s = &doom_map_secs[th->sector];
                const struct d_line *lines = &doom_map_lines[s->first_line];
                int moved = 0;
                for (uint32_t l = 0; l < s->num_lines; l++) {
                    int32_t b = lines[l].back;
                    if (b >= 0 && doom_st.secs[b].ceil_h - doom_map_secs[b].floor_h >= 16
                        && point_in_sector((uint32_t)b, th->x, th->y, 0)) {
                        th->sector = (uint8_t)b;
                        moved = 1;
                        break;
                    }
                }
                if (!moved) th->kind = TH_NONE;
            }
            if (++th->timer > 60) th->kind = TH_NONE;
            continue;
        }

        if (th->state == 3) continue;
        if (th->state == 2) { if (th->timer) th->timer--; else th->state = 1; continue; }
        if (th->state == 0) { if (dist < 420) th->state = 1; continue; }

        if (dist > 44 && dist < 600) {
            int32_t spd = (th->kind == TH_DEMON) ? 4 : 2;
            int32_t nx = th->x + ((dx > 8 ? spd : dx < -8 ? -spd : 0) << 16);
            int32_t ny = th->y + ((dy > 8 ? spd : dy < -8 ? -spd : 0) << 16);
            if (point_in_sector(th->sector, nx, ny, 10)) { th->x = nx; th->y = ny; }
        }
        if (th->timer) {
            th->timer--;
        } else if (dist < 48 && doom_st.health > 0) {
            hurt_player(th->kind == TH_DEMON ? 10 : 5);
            th->timer = 18;
        } else if (th->kind == TH_IMP && dist < 360 && doom_st.health > 0) {
            for (int32_t f = 9; f < MAX_THINGS; f++) {
                if (doom_st.things[f].kind != TH_NONE) continue;
                struct d_thing *fb = &doom_st.things[f];
                fb->kind = TH_FIREBALL;
                fb->sector = th->sector;
                fb->x = th->x;
                fb->y = th->y;
                fb->z = 26;
                fb->vx = doom_sdiv(dx << 16, dist > 1 ? dist : 1) * 9;
                fb->vy = doom_sdiv(dy << 16, dist > 1 ? dist : 1) * 9;
                fb->timer = 0;
                break;
            }
            th->timer = 34;
        } else if (th->kind == TH_ZOMBIE && dist < 300 && doom_st.health > 0) {
            if ((doom_rand() & 3u) == 0u) hurt_player(4);
            th->timer = 28;
        }
    }
}

/* ---------- 2D Drawing: Weapon, Status Bar, Mugshot & Automap (DOOM_D) ---------- */

__attribute__((section(".doom.hud")))
static void fb_rect(int32_t x, int32_t y, int32_t w, int32_t h, uint8_t c) {
    if (x < 0) { w += x; x = 0; }
    if (y < 0) { h += y; y = 0; }
    if (x + w > D_W) w = D_W - x;
    if (y + h > D_H) h = D_H - y;
    if (w <= 0 || h <= 0) return;
    for (int32_t r = 0; r < h; r++)
        memset(doom_fb + (y + r) * D_W + x, c, (size_t)w);
}

__attribute__((section(".doom.hud")))
static void draw_glyph_4x5(int32_t cx, int32_t y, uint32_t idx, uint8_t top_c) {
    uint32_t g = hud_digits[idx];
    for (int32_t r = 0; r < 5; r++) {
        uint32_t row = (g >> ((4 - r) * 4)) & 15u;
        for (int32_t col = 0; col < 4; col++)
            if (row & (8u >> col))
                fb_rect(cx + col * 2, y + r * 3, 2, 3, (r == 0) ? top_c : 114u);
    }
}

__attribute__((section(".doom.hud")))
static void draw_big_num(int32_t x, int32_t y, int32_t val, int pct) {
    uint8_t buf[3];
    int32_t n = 0;
    val = d_clamp(val, 0, 999);
    do {
        uint32_t q = doom_udiv((uint32_t)val, 10u);
        buf[n++] = (uint8_t)((uint32_t)val - q * 10u);
        val = (int32_t)q;
    } while (val > 0 && n < 3);

    int32_t cx = x - (n + (pct ? 1 : 0)) * 10;
    for (int32_t i = n - 1; i >= 0; i--, cx += 10)
        draw_glyph_4x5(cx, y, buf[i], 112u);
    if (pct) draw_glyph_4x5(cx, y, 10u, 114u);
}

__attribute__((section(".doom.hud")))
static void draw_micro_str(int32_t x, int32_t y, const char *s, uint8_t c) {
    for (; *s; s++, x += 4) {
        if (*s < 'A' || *s > 'Z') continue;
        uint16_t g = micro_font[*s - 'A'];
        for (int32_t r = 0; r < 4; r++) {
            uint32_t row = (g >> ((3 - r) * 4)) & 7u;
            for (int32_t col = 0; col < 3; col++)
                if (row & (4u >> col)) doom_fb[(y + r) * D_W + x + col] = c;
        }
    }
}

__attribute__((section(".doom.hud")))
static void draw_weapon(void) {
    int32_t cx = D_HALF_W + (d_sin(doom_st.tic << 5) >> 12);
    int32_t by = D_VIEWH + (d_cos(doom_st.tic << 6) >> 13) + (int32_t)doom_st.recoil * 2;

    if (doom_st.flash_tics) {
        int32_t fy = by - (doom_st.weapon == WPN_SHOTGUN ? 68 : 60);
        fb_rect(cx - 18, fy - 12, 36, 18, 128u);
        fb_rect(cx - 10, fy - 20, 20, 26, 129u);
        fb_rect(cx - 5,  fy - 14, 10, 14, 0u);
    }

    if (doom_st.weapon == WPN_PISTOL) {
        fb_rect(cx - 7,  by - 58, 14, 24, 5u);
        fb_rect(cx - 5,  by - 56, 10, 20, 2u);
        fb_rect(cx - 3,  by - 62,  6,  6, 10u);
        fb_rect(cx - 11, by - 36, 22, 36, 162u);
        fb_rect(cx - 13, by - 22, 26, 22, 165u);
    } else if (doom_st.weapon == WPN_SHOTGUN) {
        fb_rect(cx - 10, by - 66, 20, 32, 6u);
        fb_rect(cx - 6,  by - 68, 12, 26, 3u);
        fb_rect(cx - 3,  by - 70,  6,  4, 128u);
        fb_rect(cx - 14, by - 36, 28, 36, 52u);
        fb_rect(cx - 10, by - 28, 20, 28, 55u);
    } else {
        int32_t spin = (doom_st.tic & 1u) ? 2 : 0;
        fb_rect(cx - 22, by - 56, 44, 56, 8u);
        for (int32_t b = -16 + spin; b <= 14; b += 8) {
            fb_rect(cx + b, by - 62, 5, 42, 4u);
            fb_rect(cx + b + 1, by - 62, 2, 42, 1u);
        }
        fb_rect(cx - 24, by - 30, 48, 8, 130u);
    }
}

__attribute__((section(".doom.hud")))
static void draw_status_bar(void) {
    static const int16_t divs[4] = { 62, 136, 184, 256 };
    fb_rect(0, 200, D_W, 40, 10u);
    fb_rect(0, 200, D_W, 2, 4u);
    fb_rect(0, 238, D_W, 2, 14u);
    for (int32_t i = 0; i < 4; i++) {
        fb_rect(divs[i], 202, 2, 36, 14u);
        fb_rect(divs[i] + 2, 202, 1, 36, 6u);
    }

    draw_big_num(56,  206, doom_st.ammo,   0); draw_micro_str(18,  229, "AMMO",   3u);
    draw_big_num(130, 206, doom_st.health, 1); draw_micro_str(86,  229, "HEALTH", 3u);
    draw_big_num(250, 206, doom_st.armor,  1); draw_micro_str(206, 229, "ARMOR",  3u);
    draw_big_num(312, 206, doom_st.kills,  0); draw_micro_str(274, 229, "KILLS",  3u);

    for (uint32_t w = 0; w < 3u; w++)
        fb_rect(266 + (int32_t)w * 10, 208, 6, 8, (doom_st.weapon == w) ? 128u : 13u);

    /* Mugshot */
    int32_t fx = 146, fy = 204;
    fb_rect(fx - 2, fy - 1, 32, 34, 14u);
    fb_rect(fx, fy, 28, 32, 12u);
    fb_rect(fx + 5, fy + 2, 18, 6, 52u);
    fb_rect(fx + 4, fy + 5, 20, 4, 54u);
    fb_rect(fx + 6, fy + 8, 16, 18, 162u);
    fb_rect(fx + 8, fy + 24, 12, 4, 164u);
    if (doom_st.health < 60 || doom_st.pain_tics) {
        fb_rect(fx + 7, fy + 16, 4, 4, 114u);
        fb_rect(fx + 18, fy + 11, 3, 5, 112u);
    }
    int32_t look = (doom_st.face_expr == 0) ? -1 : (doom_st.face_expr == 2) ? 1 : 0;
    fb_rect(fx + 8,  fy + 13, 4, 3, 1u);
    fb_rect(fx + 16, fy + 13, 4, 3, 1u);
    fb_rect(fx + 9 + look,  fy + 14, 2, 2, 15u);
    fb_rect(fx + 17 + look, fy + 14, 2, 2, 15u);
    if (doom_st.face_expr == 3) fb_rect(fx + 9, fy + 21, 10, 3, 1u);
    else if (doom_st.face_expr == 4) fb_rect(fx + 11, fy + 20, 6, 5, 114u);
    else fb_rect(fx + 10, fy + 22, 8, 2, 66u);

    if (doom_st.won) {
        fb_rect(96, 82, 128, 32, 14u);
        fb_rect(98, 84, 124, 28, 118u);
        draw_micro_str(122, 92, "LEVEL CLEAR", 128u);
        draw_micro_str(116, 102, "PRESS QS PLAY", 0u);
    }
}

__attribute__((section(".doom.hud")))
static void am_line(int32_t x0, int32_t y0, int32_t x1, int32_t y1, uint8_t c) {
    int32_t dx = x1 - x0, dy = y1 - y0;
    int32_t sx = dx >= 0 ? 1 : -1, sy = dy >= 0 ? 1 : -1;
    dx = d_abs(dx);
    dy = d_abs(dy);
    int32_t err = dx - dy;
    for (int32_t steps = 0; steps < 400; steps++) {
        if ((uint32_t)x0 < D_W && (uint32_t)y0 < D_VIEWH)
            doom_fb[y0 * D_W + x0] = c;
        if (x0 == x1 && y0 == y1) break;
        int32_t e2 = err << 1;
        if (e2 > -dy) { err -= dy; x0 += sx; }
        if (e2 < dx)  { err += dx; y0 += sy; }
    }
}

__attribute__((section(".doom.hud")))
static void draw_automap(void) {
    fb_rect(0, 0, D_W, D_VIEWH, 15u);
    int32_t cx = doom_st.px >> 16, cy = doom_st.py >> 16;
    for (uint32_t i = 0; i < DOOM_LINES; i++) {
        const struct d_line *ln = &doom_map_lines[i];
        uint8_t col = (ln->flags & LINE_EXIT) ? 96u
                    : (ln->flags & LINE_DOOR) ? 144u
                    : (ln->back < 0) ? 112u : 130u;
        am_line(D_HALF_W + ((ln->x1 - cx) >> 2), D_HALF_H - ((ln->y1 - cy) >> 2),
                D_HALF_W + ((ln->x2 - cx) >> 2), D_HALF_H - ((ln->y2 - cy) >> 2), col);
    }
    for (int32_t i = 0; i < MAX_THINGS; i++) {
        const struct d_thing *th = &doom_st.things[i];
        if (th->kind == TH_NONE || th->state == 3) continue;
        fb_rect(D_HALF_W + (((th->x >> 16) - cx) >> 2) - 1,
                D_HALF_H - (((th->y >> 16) - cy) >> 2) - 1,
                3, 3, (th->kind <= TH_DEMON) ? 112u : 96u);
    }
    int32_t fx = (d_cos(doom_st.angle) * 10) >> 14, fy = (d_sin(doom_st.angle) * 10) >> 14;
    am_line(D_HALF_W - (fx >> 1), D_HALF_H + (fy >> 1), D_HALF_W + fx, D_HALF_H - fy, 0u);
    fb_rect(D_HALF_W - 1, D_HALF_H - 1, 3, 3, 0u);
    draw_micro_str(8, 8, "E HANGAR", 128u);
}

#define PSAVE_DOOM_MASK 0x40000000u /* Exclusive power-save inhibit bit */
#define DOOM_MBX_UI     0x11
#define DOOM_MPF_UI     0x10
#define APP_ACTIVE_TAG  0x44324131u /* "D2A1" */

static struct {
    volatile uint32_t active_magic;
    dp2m_app_entry_fn entry;
    uint8_t dial_prev;
} app_rt;

static void app_present_argb(void) {
    hw_dcache_sync();
    stock_flush_rect(0, 0, 640, 480);
    stock_argb_commit();
}

static void doom_present(void) {
    if (doom_st.automap) {
        draw_automap();
    } else {
        doom_render_3d();
        draw_weapon();
    }
    draw_status_bar();

    const uint32_t *pal = doom_st.pain_tics  ? doom_pal32[1]
                        : doom_st.bonus_tics ? doom_pal32[2]
                        : doom_pal32[0];
    doom_upscale_2x2(doom_fb, ARGB_FB, pal);
    app_present_argb();
}

__attribute__((noinline, section(".doom.hud")))
static uint32_t app_get_ms(void) {
    uint64_t t __attribute__((aligned(8))) = 0u;
    stock_get_tim(&t);
    if (t == 0u) {
        uint32_t h1 = REG32(0xA07F3788u);
        uint32_t lo = REG32(0xA07F378Cu);
        uint32_t h2 = REG32(0xA07F3788u);
        if (h1 != h2) lo = REG32(0xA07F378Cu);
        t = ((uint64_t)h2 << 32) | lo;
    }
    return (uint32_t)t;
}

__attribute__((noinline, section(".doom.hud")))
static void app_sleep_ms(uint32_t ms) {
    if (ms) stock_dly_tsk(ms);
}

__attribute__((noinline, section(".doom.hud")))
static int32_t app_mdf_read(int32_t fd, void *buf, uint32_t len, uint32_t *got) {
    hw_dcache_sync();
    int32_t rc = stock_mdf_read(fd, buf, len, got);
    hw_dcache_sync();
    return rc;
}

__attribute__((noinline, section(".doom.hud")))
static void app_poll_input(struct dp2m_input_snapshot *out) {
    /* Stock polarity (0xA016D88C..0xA016DB64, 0xA016E47A; SW28 is assembled by 0xA016DD94):
     * - 0xA07FAA1C (KEY0): 0x00000000 when idle; 2-bit slot (k0 >> (30 - 2*k)) & 3
     *   is 1 on press and 3 on auto-repeat for k = 0..11 (OK,UP,DN,LF,RT,MENU,DISP,AEL,QS,MODE,DEL,PLAY).
     * - 0xA07FAA28 (SW28): bit 31 = half-press (S1 only), bit 26 = full press (S1 and S2). Bit 30 mirrors an
     *   unrelated active-low line and reads 1 at idle on camera (probe V2: SW28 = 0x40000000), so it must
     *   stay out of the shutter mask. */
    uint32_t k0 = REG32(0xA07FAA1Cu);
    uint32_t sw = REG32(0xA07FAA28u);
    uint32_t dial = REG8(0xA07AF38Cu);
    uint32_t mask = 0u;
    for (uint32_t k = 0u; k < 12u; k++) {
        if (((k0 >> (30u - 2u * k)) & 3u) != 0u)
            mask |= (1u << k);
    }
    if ((sw & DP2M_SW28_SHUTTER) != 0u)
        mask |= DP2M_BTN_SHUTTER;

    int32_t d_delta = 0;
    if (dial >= 1u && dial <= 3u) {
        uint32_t prev = app_rt.dial_prev;
        if (prev >= 1u && prev <= 3u && dial != prev) {
            uint32_t cw = (prev == 3u) ? 1u : (prev + 1u);
            d_delta = (dial == cw) ? 1 : -1;
        }
        app_rt.dial_prev = (uint8_t)dial;
    }

    out->held_mask = mask;
    out->dial_delta = d_delta;
    out->accel_x = (int8_t)stock_accel_x();
    out->accel_y = (int8_t)stock_accel_y();
    out->raw_key0 = k0;
    out->raw_sw28 = sw;
    out->raw_dial = dial;
}

static struct dp2m_app_api app_api_tbl;

void doom_boot_reset(void) {
    doom_st.magic = 0u;
    memset(&app_rt, 0, sizeof(app_rt));
}

__attribute__((noinline, section(".doom.init")))
static const struct dp2m_app_api *app_build_api(uint32_t autonomous) {
    app_api_tbl.abi_ver = DP2M_APP_ABI_VER;
    app_api_tbl.is_autonomous = autonomous;
    app_api_tbl.argb_fb = ARGB_FB;
    app_api_tbl.present_argb = app_present_argb;
    app_api_tbl.upscale_2x2 = doom_upscale_2x2;
    app_api_tbl.get_ms = app_get_ms;
    app_api_tbl.sleep_ms = app_sleep_ms;
    app_api_tbl.poll_input = app_poll_input;
    app_api_tbl.malloc_fn = stock_malloc;
    app_api_tbl.calloc_fn = stock_calloc;
    app_api_tbl.realloc_fn = stock_realloc;
    app_api_tbl.free_fn = stock_free;
    app_api_tbl.mdf_open = stock_mdf_open;
    app_api_tbl.mdf_read = app_mdf_read;
    app_api_tbl.mdf_lseek = stock_mdf_lseek;
    app_api_tbl.mdf_fstat = stock_mdf_fstat;
    app_api_tbl.mdf_close = stock_mdf_close;
    return &app_api_tbl;
}

__attribute__((noinline, section(".doom.init")))
static int32_t doom_try_load_app(uint32_t autonomous) {
    if ((uintptr_t)__bss_end > DP2M_APP_LOAD_VA)
        return -1;
    int32_t fd = stock_mdf_open("I:\\DOOM.BIN", 0u, 0u);
    if (fd < 0)
        fd = stock_mdf_open("D:\\DOOM.BIN", 0u, 0u);
    if (fd < 0)
        return -1;

    struct dp2m_app_hdr hdr;
    uint32_t got = 0u;
    if (app_mdf_read(fd, &hdr, sizeof(hdr), &got) != 0 || got != sizeof(hdr) ||
        hdr.magic != DP2M_APP_MAGIC || hdr.abi_ver != DP2M_APP_ABI_VER ||
        hdr.load_va != DP2M_APP_LOAD_VA || hdr.image_size < sizeof(hdr) ||
        hdr.image_size > (DP2M_APP_LIMIT_VA - DP2M_APP_LOAD_VA) ||
        hdr.bss_end < hdr.load_va + hdr.image_size || hdr.bss_end > DP2M_APP_LIMIT_VA ||
        (hdr.entry_va & ~1u) < hdr.load_va || (hdr.entry_va & ~1u) >= hdr.load_va + hdr.image_size) {
        stock_mdf_close(fd);
        return -1;
    }

    uint32_t pos = 0u;
    if (stock_mdf_lseek(fd, 0, 0u, &pos) != 0) {
        stock_mdf_close(fd);
        return -1;
    }

    uint8_t *dst = (uint8_t *)(uintptr_t)hdr.load_va;
    uint32_t rem = hdr.image_size;
    while (rem > 0u) {
        uint32_t chunk = rem > 65536u ? 65536u : rem;
        got = 0u;
        if (app_mdf_read(fd, dst, chunk, &got) != 0 || got == 0u || got > chunk) {
            stock_mdf_close(fd);
            return -1;
        }
        dst += got;
        rem -= got;
    }
    stock_mdf_close(fd);

    uint32_t bss_len = hdr.bss_end - (hdr.load_va + hdr.image_size);
    if (bss_len)
        memset((void *)(uintptr_t)(hdr.load_va + hdr.image_size), 0, bss_len);

    hw_dcache_sync();
    hw_icache_inv_all();

    app_rt.dial_prev = REG8(0xA07AF38Cu);
    dp2m_app_entry_fn entry = (dp2m_app_entry_fn)(uintptr_t)hdr.entry_va;
    const struct dp2m_app_api *api = app_build_api(autonomous);
    if (entry(api, DP2M_APP_INIT, 0u) != 0) {
        return -1;
    }
    app_rt.entry = entry;
    app_rt.active_magic = APP_ACTIVE_TAG;
    return 0;
}

static void doom_clear_planes(void) {
    stock_argb_clear();
    stock_argb_commit();
    stock_osd_clear();
    stock_osd_commit();
}

__attribute__((noinline, section(".doom.hud")))
static void doom_shutdown_app_if_active(void) {
    if (app_rt.active_magic == APP_ACTIVE_TAG && app_rt.entry) {
        app_rt.entry(&app_api_tbl, DP2M_APP_SHUTDOWN, 0u);
    }
    app_rt.active_magic = 0u;
    app_rt.entry = 0;
}

/* Poll MBX 0x11 each autonomous frame so MPF 0x10 never starves and system power-off
 * packets (type == 6, 0xA004330C) break out immediately without being discarded. */
__attribute__((noinline, section(".doom.hud")))
static int doom_poll_ui_mbx(void **sys_msg, int app_active) {
    int should_exit = 0;
    void *msg = 0;
    while (stock_prcv_mbx(DOOM_MBX_UI, &msg) == 0 && msg) {
        int8_t mtype = *(const int8_t *)((const uint8_t *)msg + 8);
        uint32_t marg = *(const uint32_t *)((const uint8_t *)msg + 12);
        if (mtype == 6) {
            if (*sys_msg == 0)
                *sys_msg = msg;
            else
                stock_rel_mpf(DOOM_MPF_UI, msg);
            should_exit = 1;
        } else {
            stock_rel_mpf(DOOM_MPF_UI, msg);
            if (mtype == 5 && (marg == 0x0Cu || (!app_active && marg == 0x06u)))
                should_exit = 1;
        }
        msg = 0;
    }
    return should_exit;
}

__attribute__((noinline, section(".doom.hud")))
static void doom_hw_exit_to_lv(void *sys_msg) {
    stock_psave_release(PSAVE_DOOM_MASK);
    /* Sync shutter & key edge snapshots so held buttons on exit never fire spurious LV edges */
    REG32(0xA07FAB20u) = REG32(0xA07FAA28u);
    REG32(0xA07FAB24u) = REG32(0xA07FAA1Cu);
    /* Drain any final queued UI messages accumulated in MBX 0x11 back to MPF 0x10 */
    void *msg = 0;
    while (stock_prcv_mbx(DOOM_MBX_UI, &msg) == 0 && msg) {
        int8_t mtype = *(const int8_t *)((const uint8_t *)msg + 8);
        if (mtype == 6 && !sys_msg)
            sys_msg = msg;
        else
            stock_rel_mpf(DOOM_MPF_UI, msg);
        msg = 0;
    }
    doom_clear_planes();
    if (sys_msg) {
        /* Re-queue system power-off packet (type 6) to MBX 0x11 so 0xA004330C shuts down cleanly */
        stock_snd_mbx(DOOM_MBX_UI, sys_msg);
    } else {
        stock_screen_goto(SCREEN_LV);
    }
}

/* Autonomous 35 Hz loop for the built-in E1M1 engine when SD DOOM.BIN/WAD is absent */
__attribute__((noinline, section(".doom.hud")))
static void doom_run_builtin_autonomous(void **sys_msg) {
    doom_ensure_init();
    app_rt.dial_prev = REG8(0xA07AF38Cu);
    uint32_t prev_held = 0xFFFFFFFFu;
    for (;;) {
        uint32_t t0 = app_get_ms();
        if (doom_poll_ui_mbx(sys_msg, 0))
            break;
        struct dp2m_input_snapshot inp;
        app_poll_input(&inp);
        uint32_t m = inp.held_mask;
        uint32_t pressed = m & ~prev_held;
        prev_held = m;

        if (pressed & (DP2M_BTN_PLAY | DP2M_BTN_MENU))
            break;

        int32_t ca = d_cos(doom_st.angle), sa = d_sin(doom_st.angle);
        if (m & DP2M_BTN_UP) {
            move_player(ca << 5, sa << 5);
            doom_st.pvx = ca << 4; doom_st.pvy = sa << 4;
        }
        if (m & DP2M_BTN_DOWN) {
            move_player(-(ca << 5), -(sa << 5));
            doom_st.pvx = -(ca << 4); doom_st.pvy = -(sa << 4);
        }
        if (m & DP2M_BTN_QS) {
            /* QS chord: Left/Right strafe, Dial cycles weapon */
            if (m & DP2M_BTN_LEFT)  move_player(-(sa << 5), ca << 5);
            if (m & DP2M_BTN_RIGHT) move_player(sa << 5, -(ca << 5));
            if (inp.dial_delta > 0) doom_st.weapon = (uint8_t)(doom_st.weapon >= 2u ? 0u : doom_st.weapon + 1u);
            if (inp.dial_delta < 0) doom_st.weapon = (uint8_t)(doom_st.weapon ? doom_st.weapon - 1u : 2u);
        } else {
            if (m & DP2M_BTN_LEFT)  doom_st.angle = (doom_st.angle + 16u) & ANG_MASK;
            if (m & DP2M_BTN_RIGHT) doom_st.angle = (doom_st.angle - 16u) & ANG_MASK;
            if (inp.dial_delta != 0)
                doom_st.angle = (doom_st.angle - (uint32_t)(inp.dial_delta * 28)) & ANG_MASK;
        }
        if ((m & (DP2M_BTN_SHUTTER | DP2M_BTN_OK)) && doom_st.recoil == 0u)
            doom_fire();
        if (pressed & DP2M_BTN_AEL)  doom_use();
        if (pressed & DP2M_BTN_DISP) doom_st.automap ^= 1u;
        if (pressed & DP2M_BTN_MODE) doom_st.weapon = (uint8_t)(doom_st.weapon >= 2u ? 0u : doom_st.weapon + 1u);
        if ((pressed & DP2M_BTN_QS) && doom_st.won) doom_reset_level();

        doom_step_sim();
        doom_present();

        uint32_t dt = app_get_ms() - t0;
        app_sleep_ms(dt < 28u ? (28u - dt) : 1u);
    }
}

/* ---------- M6M Screen Vtable Handler (Screen 33 = 0x21) & Menu Launcher ---------- */

uint32_t doom_screen(uint32_t msg, uint32_t arg) {
    uint16_t tid = 0u;
    stock_get_tid(&tid);
    uint32_t autonomous = (tid != 0u) ? 1u : 0u;

    if (msg == 4u) {
        if (arg == 0u || arg == 1u) {
            stock_osd_clear();
            stock_osd_commit();
            if (autonomous) {
                void *sys_msg = 0;
                stock_psave_inhibit(PSAVE_DOOM_MASK);
                if (doom_try_load_app(1u) == 0) {
                    while (!doom_poll_ui_mbx(&sys_msg, 1) &&
                           app_rt.entry(&app_api_tbl, DP2M_APP_STEP, 0u) == 0) {}
                    doom_shutdown_app_if_active();
                } else {
                    doom_run_builtin_autonomous(&sys_msg);
                }
                doom_hw_exit_to_lv(sys_msg);
                return 0u;
            }
            if (doom_try_load_app(0u) != 0) {
                doom_ensure_init();
                doom_present();
            }
        } else if (arg == 3u) {
            if (app_rt.active_magic == APP_ACTIVE_TAG && app_rt.entry) {
                if (app_rt.entry(&app_api_tbl, DP2M_APP_STEP, 0u) != 0) {
                    doom_shutdown_app_if_active();
                    doom_clear_planes();
                    stock_screen_goto(SCREEN_LV);
                }
            } else {
                doom_ensure_init();
                doom_step_sim();
                doom_present();
            }
        }
        return 0u;
    }

    if (msg == 3u) {
        doom_shutdown_app_if_active();
        doom_clear_planes();
        return 0u;
    }

    if (msg == 5u) {
        if (app_rt.active_magic == APP_ACTIVE_TAG && app_rt.entry) {
            if (app_rt.entry(&app_api_tbl, DP2M_APP_STEP, arg) != 0) {
                doom_shutdown_app_if_active();
                doom_clear_planes();
                stock_screen_goto(SCREEN_LV);
            }
            return 0u;
        }

        doom_ensure_init();
        if (arg == 0x06u || arg == 0x0Cu) {
            doom_clear_planes();
            stock_screen_goto(SCREEN_LV);
            return 0u;
        }

        int32_t ca = d_cos(doom_st.angle), sa = d_sin(doom_st.angle);
        switch (arg) {
        case 0x02u: /* Up */
            move_player(ca << 6, sa << 6);
            doom_st.pvx = ca << 4; doom_st.pvy = sa << 4;
            break;
        case 0x03u: /* Down */
            move_player(-(ca << 6), -(sa << 6));
            doom_st.pvx = -(ca << 4); doom_st.pvy = -(sa << 4);
            break;
        case 0x04u: doom_st.angle = (doom_st.angle + 32u) & ANG_MASK; break; /* Left */
        case 0x05u: doom_st.angle = (doom_st.angle - 32u) & ANG_MASK; break; /* Right */
        case 0x01u: /* Ok */
        case 0xFEu: doom_fire(); break;                                      /* Shutter */
        case 0x08u: doom_use(); break;                                       /* AEL */
        case 0x07u: doom_st.automap ^= 1u; break;                            /* DISP */
        case 0x09u: doom_reset_level(); break;                               /* QS */
        case 0x20u: doom_st.weapon = (uint8_t)(doom_st.weapon ? doom_st.weapon - 1u : 2u); break;
        case 0x40u:
        case 0x0Au: doom_st.weapon = (uint8_t)(doom_st.weapon >= 2u ? 0u : doom_st.weapon + 1u); break;
        default: break;
        }

        doom_step_sim();
        doom_present();
    }
    return 0u;
}

/* Menu row extra handler (w7 in Setup Page 4): opens SCREEN_DOOM (33 = 0x21) */
uint32_t doom_open(void) {
    /* Reference doom_screen so --gc-sections retains it iff DOOM is enabled */
    volatile uint32_t keep = (uint32_t)(uintptr_t)doom_screen;
    (void)keep;
    stock_screen_post(1u, SCREEN_DOOM);
    return 0u;
}
