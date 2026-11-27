/* Bare-metal ARM926EJ-S Big-Endian DOOM (E1M1 Hangar) for Sigma DP2 Merrill */
#include "doom.h"

#define DOOM_MAGIC 0x444F4F4Du /* "DOOM" */

/* Runtime RAM buffers in payload .bss (0xA1C00000..0xA1E00000) */
struct d_state doom_st;
uint8_t doom_fb[D_W * D_H] __attribute__((aligned(4)));
uint8_t doom_tex[TEX_COUNT][64 * 64] __attribute__((aligned(4)));
uint8_t doom_spr[7][32 * 32] __attribute__((aligned(4)));
uint8_t doom_cmap[32][256] __attribute__((aligned(4)));
uint32_t doom_pal32[3][256] __attribute__((aligned(4)));
int16_t doom_sin[1024] __attribute__((aligned(4)));

/* E1M1 Hangar Tribute: 13 strictly convex CCW sectors, 66 linedefs */
const uint8_t doom_num_secs = 13u;

const struct d_sector_def doom_map_secs[13] = {
    /* 0  Start Room (Blue Carpet!) */ {   0, 128, FLAT_BLUE,   FLAT_CEIL1, 224, 0,           0, 6 },
    /* 1  Main Foyer                */ {   0, 160, FLAT_FLOOR1, FLAT_CEIL1, 192, 0,           6, 6 },
    /* 2  Courtyard Overlook        */ {   8, 144, TEX_STEP,    FLAT_CEIL1, 224, 0,          12, 4 },
    /* 3  South Supply Alcove       */ {   0, 112, FLAT_BRIDGE, FLAT_CEIL1, 144, 0,          16, 4 },
    /* 4  Stair Step 1              */ {  12, 160, TEX_STEP,    FLAT_CEIL1, 192, 0,          20, 4 },
    /* 5  Stair Step 2              */ {  24, 168, TEX_STEP,    FLAT_CEIL1, 200, 0,          24, 4 },
    /* 6  Computer Room (Blinking!) */ {  24, 176, FLAT_FLOOR1, FLAT_CEIL1, 216, SEC_BLINK,  28, 8 },
    /* 7  South Nukage Pit          */ { -24, 184, FLAT_NUKAGE, FLAT_CEIL1, 176, SEC_NUKAGE, 36, 4 },
    /* 8  Zig-Zag Bridge            */ {  24, 184, FLAT_BRIDGE, FLAT_CEIL1, 204, 0,          40, 4 },
    /* 9  North Nukage Pit          */ { -24, 184, FLAT_NUKAGE, FLAT_CEIL1, 176, SEC_NUKAGE, 44, 4 },
    /* 10 Sliding Tech Door         */ {  24, 120, TEX_STEP,    FLAT_CEIL1, 180, SEC_DOOR,   48, 4 },
    /* 11 Exit Switch Room          */ {  32, 152, FLAT_BLUE,   FLAT_CEIL1, 224, 0,          52, 6 },
    /* 12 Outdoor Phobos Courtyard  */ {   0, 240, FLAT_GRASS,  TEX_SKY,    248, 0,          58, 8 },
};

const struct d_line doom_map_lines[66] = {
    /* S0 (0..5): Start Room */
    {    0,   64,   64,    0, -1, TEX_STARTAN, 0, 0 },
    {   64,    0,  256,    0, -1, TEX_STARTAN, 0, 0 },
    {  256,    0,  256,  256,  1, TEX_TECH,    0, 0 },
    {  256,  256,   64,  256, -1, TEX_STARTAN, 0, 0 },
    {   64,  256,    0,  192, -1, TEX_TECH,    0, 0 },
    {    0,  192,    0,   64, -1, TEX_TECH,    0, 0 },
    /* S1 (6..11): Main Foyer */
    {  256,    0,  480,    0,  3, TEX_STARTAN, 0, 0 },
    {  480,    0,  544,   64, -1, TEX_TECH,    0, 0 },
    {  544,   64,  544,  192,  4, TEX_STARTAN, 0, 0 },
    {  544,  192,  480,  256, -1, TEX_TECH,    0, 0 },
    {  480,  256,  256,  256,  2, TEX_BRICK,   0, 0 },
    {  256,  256,  256,    0,  0, TEX_TECH,    0, 0 },
    /* S2 (12..15): Courtyard Overlook */
    {  256,  256,  480,  256,  1, TEX_BRICK,   0, 0 },
    {  480,  256,  480,  320, -1, TEX_BRICK,   0, 0 },
    {  480,  320,  256,  320, 12, TEX_BRICK,   0, 0 },
    {  256,  320,  256,  256, -1, TEX_BRICK,   0, 0 },
    /* S3 (16..19): South Supply Alcove */
    {  288, -128,  448, -128, -1, TEX_TECH,    0, 0 },
    {  448, -128,  480,    0, -1, TEX_STARTAN, 0, 0 },
    {  480,    0,  256,    0,  1, TEX_STARTAN, 0, 0 },
    {  256,    0,  288, -128, -1, TEX_STARTAN, 0, 0 },
    /* S4 (20..23): Stair Step 1 */
    {  544,   64,  592,   64, -1, TEX_STARTAN, 0, 0 },
    {  592,   64,  592,  192,  5, TEX_STARTAN, 0, 0 },
    {  592,  192,  544,  192, -1, TEX_STARTAN, 0, 0 },
    {  544,  192,  544,   64,  1, TEX_STARTAN, 0, 0 },
    /* S5 (24..27): Stair Step 2 */
    {  592,   64,  640,   64, -1, TEX_STARTAN, 0, 0 },
    {  640,   64,  640,  192,  6, TEX_TECH,    0, 0 },
    {  640,  192,  592,  192, -1, TEX_STARTAN, 0, 0 },
    {  592,  192,  592,   64,  4, TEX_STARTAN, 0, 0 },
    /* S6 (28..35): Computer Room */
    {  640,   64,  704,    0, -1, TEX_TECH,    0, 0 },
    {  704,    0,  864,    0, -1, TEX_TECH,    0, 0 },
    {  864,    0,  864,   64,  7, TEX_BRICK,   0, 0 },
    {  864,   64,  864,  192,  8, TEX_STARTAN, 0, 0 },
    {  864,  192,  864,  256,  9, TEX_BRICK,   0, 0 },
    {  864,  256,  704,  256, -1, TEX_TECH,    0, 0 },
    {  704,  256,  640,  192, -1, TEX_TECH,    0, 0 },
    {  640,  192,  640,   64,  5, TEX_TECH,    0, 0 },
    /* S7 (36..39): South Nukage Pit */
    {  864,    0, 1120,    0, -1, TEX_BRICK,   0, 0 },
    { 1120,    0, 1120,  112, -1, TEX_BRICK,   0, 0 },
    { 1120,  112,  864,   64,  8, TEX_STEP,    0, 0 },
    {  864,   64,  864,    0,  6, TEX_BRICK,   0, 0 },
    /* S8 (40..43): Zig-Zag Bridge */
    {  864,   64, 1120,  112,  7, TEX_STEP,    0, 0 },
    { 1120,  112, 1120,  208, 10, TEX_DOOR,    LINE_DOOR, 0 },
    { 1120,  208,  864,  192,  9, TEX_STEP,    0, 0 },
    {  864,  192,  864,   64,  6, TEX_STARTAN, 0, 0 },
    /* S9 (44..47): North Nukage Pit */
    {  864,  192, 1120,  208,  8, TEX_STEP,    0, 0 },
    { 1120,  208, 1120,  320, -1, TEX_BRICK,   0, 0 },
    { 1120,  320,  864,  256, -1, TEX_BRICK,   0, 0 },
    {  864,  256,  864,  192,  6, TEX_BRICK,   0, 0 },
    /* S10 (48..51): Sliding Tech Door */
    { 1120,  112, 1152,  112, -1, TEX_STEP,    0, 0 },
    { 1152,  112, 1152,  208, 11, TEX_DOOR,    LINE_DOOR, 0 },
    { 1152,  208, 1120,  208, -1, TEX_STEP,    0, 0 },
    { 1120,  208, 1120,  112,  8, TEX_DOOR,    LINE_DOOR, 0 },
    /* S11 (52..57): Exit Switch Room */
    { 1152,  112, 1216,   64, -1, TEX_TECH,    0, 0 },
    { 1216,   64, 1344,   64, -1, TEX_STARTAN, 0, 0 },
    { 1344,   64, 1344,  256, -1, TEX_EXIT,    LINE_EXIT, 0 },
    { 1344,  256, 1216,  256, -1, TEX_STARTAN, 0, 0 },
    { 1216,  256, 1152,  208, -1, TEX_TECH,    0, 0 },
    { 1152,  208, 1152,  112, 10, TEX_DOOR,    LINE_DOOR, 0 },
    /* S12 (58..65): Outdoor Phobos Courtyard (OPEN SKY!) */
    {  192,  320,  256,  320, -1, TEX_BRICK,   0, 0 },
    {  256,  320,  480,  320,  2, TEX_BRICK,   0, 0 },
    {  480,  320,  544,  320, -1, TEX_BRICK,   0, 0 },
    {  544,  320,  544,  512, -1, TEX_BRICK,   0, 0 },
    {  544,  512,  448,  576, -1, TEX_BRICK,   0, 0 },
    {  448,  576,  288,  576, -1, TEX_BRICK,   0, 0 },
    {  288,  576,  192,  512, -1, TEX_BRICK,   0, 0 },
    {  192,  512,  192,  320, -1, TEX_BRICK,   0, 0 },
};

/* Initial thing spawns: { x, y, kind, sector, hp } */
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

/* Compact 4x5 digit + uppercase glyph bitmaps (each nibble = 4px row, 5 rows = 20 bits packed in u32) */
static const uint32_t hud_digits[11] = {
    0xF999F, /* 0 */
    0x26227, /* 1 */
    0xF1F8F, /* 2 */
    0xF1F1F, /* 3 */
    0x99F11, /* 4 */
    0xF8F1F, /* 5 */
    0xF8F9F, /* 6 */
    0xF1222, /* 7 */
    0xF9F9F, /* 8 */
    0xF9F1F, /* 9 */
    0x92490, /* % */
};

/* 3x4 micro uppercase font for status bar labels (4 rows x 3 bits packed in nibbles) */
static const uint16_t micro_font[26] = {
    0x7575, /* A */ 0x6567, /* B */ 0x7447, /* C */ 0x6556, /* D */ 0x7647, /* E */
    0x7644, /* F */ 0x7457, /* G */ 0x5755, /* H */ 0x7227, /* I */ 0x1157, /* J */
    0x5655, /* K */ 0x4447, /* L */ 0x5755, /* M */ 0x6555, /* N */ 0x7557, /* O */
    0x7574, /* P */ 0x7563, /* Q */ 0x7565, /* R */ 0x3616, /* S */ 0x7222, /* T */
    0x5557, /* U */ 0x5552, /* V */ 0x5575, /* W */ 0x5255, /* X */ 0x5222, /* Y */
    0x7127, /* Z */
};

/* Palette ramps: 8 ramps x 16 shades + sky + special colors */
static const uint8_t ramp_rgb[14][3] = {
    { 240, 240, 240 }, /*  0..15  grey / steel */
    { 196, 168, 128 }, /* 16..31  STARTAN warm tan */
    { 148, 124,  88 }, /* 32..47  olive / brown concrete */
    { 164, 108,  68 }, /* 48..63  rust / Phobos brown */
    {  96,  72,  52 }, /* 64..79  dark mountain / shadow */
    {  64, 108, 224 }, /* 80..95  E1M1 blue carpet! */
    {  64, 240,  48 }, /* 96..111 radioactive NUKAGE green */
    { 236,  44,  32 }, /* 112..127 Doom blood red / fire */
    { 252, 200,  48 }, /* 128..143 bright yellow / muzzle gold */
    {  56, 212, 224 }, /* 144..159 tech cyan screens */
    { 228, 152, 120 }, /* 160..175 skin / pinky demon */
    {  92, 128,  72 }, /* 176..191 mossy courtyard grass */
    {  84, 104, 136 }, /* 192..207 Phobos twilight sky blue-grey */
    { 208, 132,  84 }, /* 208..223 Phobos horizon amber */
};

/* ---------- Cold init & procedural asset generation (placed in DOOM_D!) ---------- */

static uint32_t doom_rand(void) {
    doom_st.rng = doom_st.rng * 1664525u + 1013904223u;
    return doom_st.rng >> 16;
}

__attribute__((section(".doom.init")))
static uint16_t rgb_to_4444(int32_t r, int32_t g, int32_t b) {
    if (r < 0) r = 0; else if (r > 255) r = 255;
    if (g < 0) g = 0; else if (g > 255) g = 255;
    if (b < 0) b = 0; else if (b > 255) b = 255;
    return (uint16_t)(((r >> 4) << 12) | ((g >> 4) << 8) | ((b >> 4) << 4) | 0xFu);
}

__attribute__((section(".doom.init")))
static void init_trig_and_pal(void) {
    /* Bhaskara/parabolic sine approximation for 1024-entry Q14 sine table (max err < 0.2%) */
    for (uint32_t i = 0; i < 256u; i++) {
        /* Quarter wave: sin(pi/2 * x) ~= x*(2 - x) refined with quadratic blend */
        /* x in 0..256 -> p = x * (512 - x) in 0..65536 */
        uint32_t p = i * (512u - i);
        /* Blend: 0.775 * p + 0.225 * p^2 / 65536 -> exact to ~0.001! */
        uint32_t p2 = (p * p) >> 16;
        int32_t s = (int32_t)((p * 198u + p2 * 58u) >> 10);
        if (s > 16384) s = 16384;
        doom_sin[i] = (int16_t)s;
        doom_sin[512u - i] = (int16_t)s;
        doom_sin[512u + i] = (int16_t)(-s);
        doom_sin[(1024u - i) & 1023u] = (int16_t)(-s);
    }
    doom_sin[256] = 16384;
    doom_sin[768] = -16384;

    /* Build 256-color PLAYPAL (normal, pain red tint, bonus gold tint) & 32-level COLORMAP */
    for (uint32_t i = 0; i < 256u; i++) {
        int32_t r = 0, g = 0, b = 0;
        if (i < 224u) {
            uint32_t ramp = i >> 4;
            uint32_t lvl = i & 15u; /* 0 = brightest, 15 = darkest */
            /* Smooth Doom lighting curve: bright -> moody shadow without premature black crush */
            uint32_t k = 256u - (lvl * 14u);
            if (ramp >= 12u) k = 128u + lvl * 8u; /* sky ramps brighten toward horizon */
            r = (ramp_rgb[ramp][0] * (int32_t)k) >> 8;
            g = (ramp_rgb[ramp][1] * (int32_t)k) >> 8;
            b = (ramp_rgb[ramp][2] * (int32_t)k) >> 8;
        }
        uint16_t c0 = rgb_to_4444(r, g, b);
        uint16_t c1 = rgb_to_4444(r + 120, g >> 1, b >> 1);
        uint16_t c2 = rgb_to_4444(r + 80, g + 64, b >> 1);
        doom_pal32[0][i] = ((uint32_t)c0 << 16) | c0;
        doom_pal32[1][i] = ((uint32_t)c1 << 16) | c1;
        doom_pal32[2][i] = ((uint32_t)c2 << 16) | c2;

        /* COLORMAP: shade 0 (bright) .. 31 (near black) shifts down within each 16-color ramp */
        for (uint32_t sh = 0; sh < 32u; sh++) {
            if (i < 192u) {
                uint32_t ramp = i & 0xF0u;
                uint32_t idx = (i & 15u) + (sh >> 1);
                if (idx > 15u) idx = 15u;
                doom_cmap[sh][i] = (uint8_t)(ramp | idx);
            } else {
                doom_cmap[sh][i] = (uint8_t)i;
            }
        }
    }
}

/* Generate 12 iconic 64x64 Doom textures & flats procedurally into .bss RAM! */
/* Note: walls are column-major tex[u*64 + v], flats are row-major flat[v*64 + u] */
__attribute__((section(".doom.init")))
static void gen_textures(void) {
    for (uint32_t u = 0; u < 64u; u++) {
        for (uint32_t v = 0; v < 64u; v++) {
            uint32_t h = ((u * 73u) ^ (v * 151u) ^ ((u + v) * 37u)) & 7u;
            uint32_t widx = (u << 6) | v;
            uint32_t fidx = (v << 6) | u;

            /* 0: TEX_STARTAN (tan metallic UAC wall panels with rivets & horizontal bands) */
            {
                uint32_t c = 18u + (h & 3u);
                if (v == 0 || v == 31 || v == 32 || v == 63) c = 28u;
                else if (v == 1 || v == 33) c = 16u;
                else if ((u & 15u) == 0u) c = 27u;
                else if ((u & 15u) == 1u) c = 17u;
                else if ((v == 4 || v == 28 || v == 36 || v == 60) && ((u & 15u) == 3u || (u & 15u) == 12u)) c = 12u; /* rivets */
                else if (v >= 54u) c = 36u + (h & 3u); /* dark lower baseboard */
                doom_tex[TEX_STARTAN][widx] = (uint8_t)c;
            }

            /* 1: TEX_TECH (dark steel UAC computer panels with glowing cyan/green/red displays) */
            {
                uint32_t c = 6u + (h & 3u);
                if ((u & 31u) == 0u || (v & 31u) == 0u) c = 14u;
                else if ((u & 31u) == 1u || (v & 31u) == 1u) c = 3u;
                else if (u >= 6u && u <= 26u && v >= 8u && v <= 24u) {
                    /* Cyan CRT console screen with scanline bars */
                    c = ((v & 3u) == 0u) ? 148u : (144u + ((u ^ v) & 3u));
                    if (u == 6u || u == 26u || v == 8u || v == 24u) c = 13u;
                } else if (u >= 38u && u <= 58u && v >= 10u && v <= 22u) {
                    /* Status indicator LEDs */
                    c = ((u & 7u) < 4u && (v & 3u) < 2u) ? (((u >> 3) & 1u) ? 96u : 112u) : 13u;
                } else if (v >= 38u && v <= 56u && (v & 3u) == 0u && u >= 6u && u <= 58u) {
                    c = 14u; /* cooling vents */
                }
                doom_tex[TEX_TECH][widx] = (uint8_t)c;
            }

            /* 2: TEX_BRICK (weathered brown/olive Phobos concrete blocks) */
            {
                uint32_t row = v >> 4;
                uint32_t bu = (u + (row & 1u) * 16u) & 31u;
                uint32_t bv = v & 15u;
                uint32_t c = 36u + ((row ^ (bu >> 4)) & 3u) + (h & 3u);
                if (bv == 0u || bu == 0u) c = 46u;
                else if (bv == 1u || bu == 1u) c = 33u;
                doom_tex[TEX_BRICK][widx] = (uint8_t)c;
            }

            /* 3: TEX_DOOR (heavy ribbed UAC steel blast door + yellow/black caution trim) */
            {
                uint32_t c = 4u + (h & 1u);
                if (v >= 56u) {
                    /* Caution stripes along bottom edge of door */
                    c = (((u + v) >> 3) & 1u) ? 130u : 14u;
                } else if (u < 6u || u >= 58u) {
                    c = 10u + (v & 1u);
                } else if (u == 31u || u == 32u) {
                    c = 14u; /* center seam */
                } else if ((v & 7u) == 0u) {
                    c = 11u;
                } else if (v >= 22u && v <= 42u && u >= 24u && u <= 40u) {
                    c = (u == 24u || u == 40u || v == 22u || v == 42u) ? 13u : 114u; /* red lock emblem */
                }
                doom_tex[TEX_DOOR][widx] = (uint8_t)c;
            }

            /* 4: TEX_EXIT (glowing UAC red/yellow EXIT switch panel!) */
            {
                uint32_t c = 8u + (h & 3u);
                if (u >= 10u && u <= 54u && v >= 10u && v <= 54u) {
                    if (u == 10u || u == 54u || v == 10u || v == 54u) c = 130u;
                    else if (v >= 16u && v <= 26u) c = ((u & 3u) != 0u) ? 112u : 120u; /* red header bar */
                    else if (u >= 22u && u <= 42u && v >= 32u && v <= 48u) {
                        /* Glowing switch lever */
                        c = (v < 40u) ? 128u : 114u;
                    } else c = 12u;
                }
                doom_tex[TEX_EXIT][widx] = (uint8_t)c;
            }

            /* 5: TEX_STEP (industrial stair riser / metal tread) */
            {
                uint32_t c = 8u + ((u >> 2) & 3u);
                if (v < 4u) c = 130u; /* yellow safety nosing */
                else if ((v & 7u) == 0u) c = 14u;
                doom_tex[TEX_STEP][widx] = (uint8_t)c;
            }

            /* 6: FLAT_BLUE (iconic E1M1 royal blue patterned carpet!) */
            {
                uint32_t pat = ((u & 7u) == 0u || (v & 7u) == 0u) ? 6u : (((u ^ v) & 3u));
                doom_tex[FLAT_BLUE][fidx] = (uint8_t)(82u + pat);
            }

            /* 7: FLAT_FLOOR1 (industrial dark steel floor tiles) */
            {
                uint32_t c = 9u + (h & 1u);
                if ((u & 31u) == 0u || (v & 31u) == 0u) c = 14u;
                else if ((u & 31u) == 1u || (v & 31u) == 1u) c = 6u;
                doom_tex[FLAT_FLOOR1][fidx] = (uint8_t)c;
            }

            /* 8: FLAT_CEIL1 (tech ceiling with glowing light panels!) */
            {
                uint32_t c = 10u + (h & 1u);
                if (u >= 20u && u <= 44u && v >= 20u && v <= 44u) {
                    c = (u == 20u || u == 44u || v == 20u || v == 44u) ? 13u : 0u; /* bright white lamp! */
                } else if ((u & 15u) == 0u || (v & 15u) == 0u) c = 14u;
                doom_tex[FLAT_CEIL1][fidx] = (uint8_t)c;
            }

            /* 9: FLAT_NUKAGE (radioactive bubbling green slime!) */
            {
                int32_t s1 = doom_sin[((u << 4) + (v << 3)) & 1023u];
                int32_t s2 = doom_sin[((v << 4) - (u << 3)) & 1023u];
                uint32_t swirl = (uint32_t)((s1 + s2 + 32768) >> 13);
                doom_tex[FLAT_NUKAGE][fidx] = (uint8_t)(96u + (swirl & 7u));
            }

            /* 10: FLAT_GRASS (Phobos outdoor gravel & moss) */
            {
                uint32_t c = ((h & 3u) == 0u) ? (178u + (h & 3u)) : (36u + (h & 3u));
                doom_tex[FLAT_GRASS][fidx] = (uint8_t)c;
            }

            /* 11: FLAT_BRIDGE (ribbed steel catwalk grating) */
            {
                uint32_t c = ((u & 7u) < 2u || (v & 7u) < 2u) ? 6u : 13u;
                if (u == 0u || u == 63u) c = 132u;
                doom_tex[FLAT_BRIDGE][fidx] = (uint8_t)c;
            }
        }
    }
}

/* Paint procedural 32x32 column-major sprites into .bss RAM (0 = transparent!) */
__attribute__((section(".doom.init")))
static void gen_sprites(void) {
    memset(doom_spr, 0, sizeof doom_spr);
    for (int32_t x = 0; x < 32; x++) {
        for (int32_t y = 0; y < 32; y++) {
            int32_t dx = x - 16, dy = y - 16;
            int32_t r2 = dx * dx + dy * dy;
            uint32_t idx = (uint32_t)((x << 5) | y);

            /* 0: TH_ZOMBIE (former human trooper: green hair, tan armor, rifle, red eyes) */
            {
                uint8_t c = 0;
                if (y >= 3 && y <= 9 && dx >= -3 && dx <= 3) c = (y <= 5) ? 100u : 164u; /* head */
                if (y == 6 && (dx == -1 || dx == 2)) c = 112u; /* glowing red eyes */
                if (y >= 10 && y <= 21 && dx >= -6 && dx <= 6) c = 20u + (uint8_t)((x ^ y) & 3u); /* tan uniform */
                if (y >= 13 && y <= 16 && dx >= 3 && dx <= 10) c = 8u; /* rifle barrel */
                if (y >= 22 && y <= 31 && ((dx >= -5 && dx <= -1) || (dx >= 1 && dx <= 5))) c = 38u; /* boots/legs */
                doom_spr[0][idx] = c;
            }

            /* 1: TH_IMP (spiky brown demon with glowing yellow eyes & fireball hands) */
            {
                uint8_t c = 0;
                if (y >= 2 && y <= 9 && dx >= -4 && dx <= 4) c = 52u + (uint8_t)((x ^ y) & 3u);
                if (y <= 4 && (dx == -4 || dx == 4)) c = 18u; /* horns/spikes */
                if (y == 6 && (dx == -2 || dx == 2)) c = 128u; /* bright yellow eyes */
                if (y >= 10 && y <= 22 && dx >= -7 && dx <= 7) c = 54u + (uint8_t)((y >> 1) & 3u);
                if (y >= 11 && y <= 16 && (dx <= -7 || dx >= 7) && dx >= -11 && dx <= 11) c = 114u; /* glowing claws */
                if (y >= 23 && y <= 31 && ((dx >= -6 && dx <= -2) || (dx >= 2 && dx <= 6))) c = 56u;
                doom_spr[1][idx] = c;
            }

            /* 2: TH_DEMON (bulky Pinky Demon: huge pink/flesh head, white horns, jaw) */
            {
                uint8_t c = 0;
                if (y >= 4 && y <= 18 && dx >= -10 && dx <= 10) c = 162u + (uint8_t)((x ^ y) & 3u);
                if (y >= 2 && y <= 6 && (dx == -9 || dx == -8 || dx == 8 || dx == 9)) c = 2u; /* horns */
                if (y == 9 && (dx == -4 || dx == 4)) c = 128u; /* eyes */
                if (y >= 13 && y <= 16 && dx >= -5 && dx <= 5) c = ((x & 1) ? 2u : 114u); /* teeth/maw */
                if (y >= 19 && y <= 31 && ((dx >= -9 && dx <= -3) || (dx >= 3 && dx <= 9))) c = 166u;
                doom_spr[2][idx] = c;
            }

            /* 3: TH_MEDIKIT (white box with bold red cross!) */
            {
                uint8_t c = 0;
                if (x >= 5 && x <= 26 && y >= 10 && y <= 28) {
                    c = (x == 5 || x == 26 || y == 10 || y == 28) ? 10u : 2u;
                    if ((dx >= -2 && dx <= 2 && y >= 14 && y <= 24) ||
                        (dx >= -6 && dx <= 6 && y >= 17 && y <= 21)) c = 112u;
                }
                doom_spr[3][idx] = c;
            }

            /* 4: TH_AMMO (olive UAC ammo crate with yellow stripes) */
            {
                uint8_t c = 0;
                if (x >= 6 && x <= 25 && y >= 12 && y <= 29) {
                    c = (x == 6 || x == 25 || y == 12 || y == 29) ? 44u : 178u;
                    if (y >= 16 && y <= 20 && x >= 9 && x <= 22) c = 128u;
                }
                doom_spr[4][idx] = c;
            }

            /* 5: TH_ARMOR (iconic Doom Green Security Armor vest!) */
            {
                uint8_t c = 0;
                if (y >= 8 && y <= 27 && dx >= -8 && dx <= 8) {
                    if (!(y <= 12 && dx >= -3 && dx <= 3))
                        c = (dx == -8 || dx == 8 || y == 27) ? 106u : (98u + (uint8_t)((y >> 2) & 3u));
                }
                doom_spr[5][idx] = c;
            }

            /* 6: TH_FIREBALL (blazing Imp plasma/fireball!) */
            {
                uint8_t c = 0;
                if (r2 < 24) c = 128u;
                else if (r2 < 64) c = 112u;
                else if (r2 < 100 && ((x ^ y) & 1)) c = 116u;
                doom_spr[6][idx] = c;
            }
        }
    }
}

__attribute__((section(".doom.init")))
static void doom_reset_level(void) {
    doom_st.px = 88 << 16;
    doom_st.py = 128 << 16;
    doom_st.pz = 0;
    doom_st.pvx = 0;
    doom_st.pvy = 0;
    doom_st.view_z = 41;
    doom_st.angle = 0;
    doom_st.sector = 0;
    doom_st.weapon = WPN_PISTOL;
    doom_st.automap = 0;
    doom_st.won = 0;
    doom_st.health = 100;
    doom_st.armor = 0;
    doom_st.ammo = 50;
    doom_st.kills = 0;
    doom_st.total_enemies = 0;
    doom_st.flash_tics = 0;
    doom_st.recoil = 0;
    doom_st.pain_tics = 0;
    doom_st.bonus_tics = 0;
    doom_st.face_expr = 1;
    doom_st.face_timer = 12;
    doom_st.tic = 0;

    for (uint32_t i = 0; i < doom_num_secs; i++) {
        const struct d_sector_def *sd = &doom_map_secs[i];
        struct d_sector *s = &doom_st.secs[i];
        s->floor_h = sd->floor_h;
        s->base_ceil = sd->ceil_h;
        s->ceil_h = (sd->flags & SEC_DOOR) ? sd->floor_h : sd->ceil_h;
        s->floor_tex = sd->floor_tex;
        s->ceil_tex = sd->ceil_tex;
        s->light = sd->light;
        s->flags = sd->flags;
        s->first_line = sd->first_line;
        s->num_lines = sd->num_lines;
        s->door_state = 0;
        s->door_timer = 0;
    }

    memset(doom_st.things, 0, sizeof doom_st.things);
    for (uint32_t i = 0; i < 9u; i++) {
        struct d_thing *th = &doom_st.things[i];
        th->x = (int32_t)init_things[i].x << 16;
        th->y = (int32_t)init_things[i].y << 16;
        th->z = 0;
        th->kind = init_things[i].kind;
        th->sector = init_things[i].sec;
        th->hp = init_things[i].hp;
        th->state = 0;
        th->timer = (uint8_t)(10u + i * 3u);
        if (th->kind <= TH_DEMON) doom_st.total_enemies++;
    }
}

__attribute__((section(".doom.init")))
static void doom_ensure_init(void) {
    if (doom_st.magic == DOOM_MAGIC) return;
    memset(&doom_st, 0, sizeof doom_st);
    doom_st.magic = DOOM_MAGIC;
    doom_st.rng = 0x19931210u; /* Dec 10, 1993 */
    init_trig_and_pal();
    gen_textures();
    gen_sprites();
    doom_reset_level();
}

/* ---------- Geometry, Collision & Gameplay (DOOM_B) ---------- */

/* Check if (x_q16, y_q16) is inside convex CCW sector `sec_id` with margin `pad` map units */
static int point_in_sector(uint32_t sec_id, int32_t x, int32_t y, int32_t pad) {
    const struct d_sector *s = &doom_st.secs[sec_id];
    const struct d_line *lines = &doom_map_lines[s->first_line];
    int32_t mx = x >> 12, my = y >> 12; /* Q4 */
    for (uint32_t i = 0; i < s->num_lines; i++) {
        int32_t x1 = (int32_t)lines[i].x1 << 4, y1 = (int32_t)lines[i].y1 << 4;
        int32_t ex = ((int32_t)lines[i].x2 << 4) - x1;
        int32_t ey = ((int32_t)lines[i].y2 << 4) - y1;
        /* Left-hand normal for CCW edge: cross(edge, pt - v1) >= 0 means inside */
        int32_t cross = (ex >> 4) * (my - y1) - (ey >> 4) * (mx - x1);
        if (pad > 0) {
            int32_t aex = ex < 0 ? -ex : ex, aey = ey < 0 ? -ey : ey;
            int32_t len = (aex > aey ? aex + (aey >> 1) : aey + (aex >> 1)) >> 4;
            if (cross < pad * (len << 4)) return 0;
        } else if (cross < 0) {
            return 0;
        }
    }
    return 1;
}

/* Try moving player from (px, py) by (dx, dy) with sector portal traversal & wall sliding */
static int try_move_player(int32_t nx, int32_t ny) {
    int32_t cur_flr = doom_st.secs[doom_st.sector].floor_h;
    /* Check current sector first, then immediate neighbors */
    if (point_in_sector(doom_st.sector, nx, ny, 0)) {
        /* Keep 14-unit clearance from solid walls & closed doors */
        const struct d_sector *s = &doom_st.secs[doom_st.sector];
        const struct d_line *lines = &doom_map_lines[s->first_line];
        int32_t mx = nx >> 12, my = ny >> 12;
        for (uint32_t i = 0; i < s->num_lines; i++) {
            int blocked = (lines[i].back < 0);
            if (!blocked) {
                const struct d_sector *bs = &doom_st.secs[lines[i].back];
                if (bs->ceil_h - bs->floor_h < 48 || bs->floor_h > cur_flr + 24) blocked = 1;
            }
            if (!blocked) continue;
            int32_t x1 = (int32_t)lines[i].x1 << 4, y1 = (int32_t)lines[i].y1 << 4;
            int32_t ex = ((int32_t)lines[i].x2 << 4) - x1;
            int32_t ey = ((int32_t)lines[i].y2 << 4) - y1;
            int32_t aex = ex < 0 ? -ex : ex, aey = ey < 0 ? -ey : ey;
            int32_t len = (aex > aey ? aex + (aey >> 1) : aey + (aex >> 1)) >> 4;
            int32_t cross = (ex >> 4) * (my - y1) - (ey >> 4) * (mx - x1);
            if (cross < 14 * (len << 4)) return 0;
        }
        doom_st.px = nx;
        doom_st.py = ny;
        return 1;
    }

    /* Check neighboring portal sectors */
    const struct d_sector *s = &doom_st.secs[doom_st.sector];
    const struct d_line *lines = &doom_map_lines[s->first_line];
    for (uint32_t i = 0; i < s->num_lines; i++) {
        int32_t b = lines[i].back;
        if (b < 0) continue;
        const struct d_sector *bs = &doom_st.secs[b];
        if (bs->ceil_h - bs->floor_h < 48 || bs->floor_h > cur_flr + 24) continue;
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
    if (!try_move_player(doom_st.px + dx, doom_st.py + dy)) {
        if (!try_move_player(doom_st.px + dx, doom_st.py))
            try_move_player(doom_st.px, doom_st.py + dy);
    }
}

/* Activate nearby door or EXIT switch (AEL / Use key) */
__attribute__((section(".doom.extra")))
static void doom_use(void) {
    const struct d_sector *s = &doom_st.secs[doom_st.sector];
    const struct d_line *lines = &doom_map_lines[s->first_line];
    int32_t px = doom_st.px >> 16, py = doom_st.py >> 16;

    for (uint32_t i = 0; i < s->num_lines; i++) {
        int32_t mx = ((int32_t)lines[i].x1 + lines[i].x2) >> 1;
        int32_t my = ((int32_t)lines[i].y1 + lines[i].y2) >> 1;
        int32_t dx = mx - px, dy = my - py;
        int32_t dist = (dx < 0 ? -dx : dx) + (dy < 0 ? -dy : dy);
        if (dist > 140) continue;

        if (lines[i].flags & LINE_EXIT) {
            doom_st.won = 1;
            doom_st.bonus_tics = 10;
            doom_st.face_expr = 3;
            doom_st.face_timer = 20;
            return;
        }
        if ((lines[i].flags & LINE_DOOR) && lines[i].back >= 0) {
            struct d_sector *ds = &doom_st.secs[lines[i].back];
            if (ds->door_state == 0 || ds->door_state == 3) {
                ds->door_state = 1; /* opening */
            }
        }
    }
    /* Also allow opening S10 door if standing close in S8 or S11 */
    if (doom_st.sector == 8 || doom_st.sector == 11) {
        if (doom_st.secs[10].door_state == 0) doom_st.secs[10].door_state = 1;
    }
}

/* Fire active weapon (hitscan with auto-aim & spread!) */
__attribute__((section(".doom.extra")))
static void doom_fire(void) {
    if (doom_st.ammo <= 0 || doom_st.health <= 0) return;
    int32_t cost = (doom_st.weapon == WPN_SHOTGUN) ? 2 : 1;
    int32_t dmg  = (doom_st.weapon == WPN_SHOTGUN) ? 45 : (doom_st.weapon == WPN_CHAINGUN) ? 22 : 18;
    int32_t cone = (doom_st.weapon == WPN_SHOTGUN) ? 80 : 48;

    doom_st.ammo -= (int16_t)cost;
    if (doom_st.ammo < 0) doom_st.ammo = 0;
    doom_st.flash_tics = 3;
    doom_st.recoil = (doom_st.weapon == WPN_SHOTGUN) ? 10 : 5;

    int32_t cos_a = d_cos(doom_st.angle);
    int32_t sin_a = d_sin(doom_st.angle);
    int32_t best = -1, best_tz = 0x7FFFFFFF;

    for (int32_t i = 0; i < MAX_THINGS; i++) {
        struct d_thing *th = &doom_st.things[i];
        if (th->kind == TH_NONE || th->kind > TH_DEMON || th->state == 3) continue;
        int32_t dx = (th->x - doom_st.px) >> 12;
        int32_t dy = (th->y - doom_st.py) >> 12;
        int32_t tz = (dx * cos_a + dy * sin_a) >> 14;
        int32_t tx = (dx * sin_a - dy * cos_a) >> 14;
        if (tz < 32 || tz > 9000) continue;
        int32_t atx = tx < 0 ? -tx : tx;
        if (atx * 256 < tz * cone && tz < best_tz) {
            best = i;
            best_tz = tz;
        }
    }

    if (best >= 0) {
        struct d_thing *th = &doom_st.things[best];
        th->hp -= (int16_t)dmg;
        if (th->hp <= 0) {
            th->hp = 0;
            th->state = 3; /* dead */
            doom_st.kills++;
            doom_st.face_expr = 3; /* grin! */
            doom_st.face_timer = 14;
        } else {
            th->state = 2; /* pain */
            th->timer = 4;
        }
    }
}

static void hurt_player(int32_t dmg) {
    if (doom_st.health <= 0) return;
    if (doom_st.armor > 0) {
        int32_t abs_d = dmg >> 1;
        if (abs_d > doom_st.armor) abs_d = doom_st.armor;
        doom_st.armor -= (int16_t)abs_d;
        dmg -= abs_d;
    }
    doom_st.health -= (int16_t)dmg;
    if (doom_st.health < 0) doom_st.health = 0;
    doom_st.pain_tics = 5;
    doom_st.face_expr = 4;
    doom_st.face_timer = 8;
}

/* Advance simulation by one game tic */
static void doom_step_sim(void) {
    doom_st.tic++;

    /* Smooth momentum & eye height interpolation over stairs */
    if (doom_st.pvx || doom_st.pvy) {
        move_player(doom_st.pvx, doom_st.pvy);
        doom_st.pvx >>= 1;
        doom_st.pvy >>= 1;
    }
    int32_t target_z = doom_st.secs[doom_st.sector].floor_h + 41;
    doom_st.view_z += (target_z - doom_st.view_z + (target_z > doom_st.view_z ? 1 : -1)) >> 1;

    /* Decay flashes & timers */
    if (doom_st.flash_tics) doom_st.flash_tics--;
    if (doom_st.recoil) doom_st.recoil = (uint8_t)(doom_st.recoil > 2 ? doom_st.recoil - 2 : 0);
    if (doom_st.pain_tics) doom_st.pain_tics--;
    if (doom_st.bonus_tics) doom_st.bonus_tics--;

    /* Doomguy mugshot glancing */
    if (doom_st.face_timer) {
        doom_st.face_timer--;
    } else {
        uint32_t r = doom_rand() & 3u;
        doom_st.face_expr = (uint8_t)(r == 3u ? 1u : r);
        doom_st.face_timer = (uint8_t)(10u + (doom_rand() & 15u));
    }

    /* Animate sliding doors */
    for (uint32_t i = 0; i < doom_num_secs; i++) {
        struct d_sector *s = &doom_st.secs[i];
        if (!(s->flags & SEC_DOOR)) continue;
        if (s->door_state == 1) {
            s->ceil_h += 8;
            if (s->ceil_h >= s->base_ceil) {
                s->ceil_h = s->base_ceil;
                s->door_state = 2;
                s->door_timer = 90;
            }
        } else if (s->door_state == 2) {
            if (s->door_timer) s->door_timer--;
            else if (doom_st.sector != i) s->door_state = 3;
        } else if (s->door_state == 3) {
            if (doom_st.sector == i) { s->door_state = 1; continue; }
            s->ceil_h -= 6;
            if (s->ceil_h <= s->floor_h) {
                s->ceil_h = s->floor_h;
                s->door_state = 0;
            }
        }
    }

    /* Nukage damage */
    if ((doom_st.secs[doom_st.sector].flags & SEC_NUKAGE) && (doom_st.tic & 15u) == 0u)
        hurt_player(5);

    /* Update enemies, pickups & fireballs */
    for (int32_t i = 0; i < MAX_THINGS; i++) {
        struct d_thing *th = &doom_st.things[i];
        if (th->kind == TH_NONE) continue;

        int32_t dx = (doom_st.px - th->x) >> 16;
        int32_t dy = (doom_st.py - th->y) >> 16;
        int32_t adx = dx < 0 ? -dx : dx, ady = dy < 0 ? -dy : dy;
        int32_t dist = adx > ady ? adx + (ady >> 1) : ady + (adx >> 1);

        /* Pickups */
        if (th->kind >= TH_MEDIKIT && th->kind <= TH_ARMOR) {
            if (dist < 32) {
                if (th->kind == TH_MEDIKIT && doom_st.health < 100) {
                    doom_st.health = doom_st.health + 25 > 100 ? 100 : doom_st.health + 25;
                } else if (th->kind == TH_AMMO) {
                    doom_st.ammo += 24;
                } else if (th->kind == TH_ARMOR) {
                    doom_st.armor = 100;
                } else {
                    continue;
                }
                th->kind = TH_NONE;
                doom_st.bonus_tics = 5;
                doom_st.face_expr = 3;
                doom_st.face_timer = 12;
            }
            continue;
        }

        /* Imp fireballs */
        if (th->kind == TH_FIREBALL) {
            th->x += th->vx;
            th->y += th->vy;
            if (dist < 22) {
                hurt_player(8);
                th->kind = TH_NONE;
                continue;
            }
            if (!point_in_sector(th->sector, th->x, th->y, 0)) {
                /* Try stepping into neighbor sector */
                const struct d_sector *s = &doom_st.secs[th->sector];
                const struct d_line *lines = &doom_map_lines[s->first_line];
                int moved = 0;
                for (uint32_t l = 0; l < s->num_lines; l++) {
                    if (lines[l].back >= 0 && point_in_sector((uint32_t)lines[l].back, th->x, th->y, 0)) {
                        th->sector = (uint8_t)lines[l].back;
                        moved = 1;
                        break;
                    }
                }
                if (!moved) th->kind = TH_NONE;
            }
            if (++th->timer > 60) th->kind = TH_NONE;
            continue;
        }

        /* Enemies (Zombieman, Imp, Pinky Demon) */
        if (th->state == 3) continue; /* dead */
        if (th->state == 2) {
            if (th->timer) th->timer--;
            else th->state = 1;
            continue;
        }
        if (th->state == 0) {
            if (dist < 420) th->state = 1;
            continue;
        }

        /* Chase & attack */
        if (dist > 44 && dist < 600) {
            int32_t spd = (th->kind == TH_DEMON) ? 4 : 2;
            int32_t step_x = (dx > 8 ? spd : dx < -8 ? -spd : 0) << 16;
            int32_t step_y = (dy > 8 ? spd : dy < -8 ? -spd : 0) << 16;
            int32_t nx = th->x + step_x, ny = th->y + step_y;
            if (point_in_sector(th->sector, nx, ny, 10)) {
                th->x = nx;
                th->y = ny;
            }
        }
        if (th->timer) {
            th->timer--;
        } else if (dist < 48 && doom_st.health > 0) {
            hurt_player(th->kind == TH_DEMON ? 10 : 5);
            th->timer = 18;
        } else if (th->kind == TH_IMP && dist < 360 && doom_st.health > 0) {
            /* Spawn fireball in free slot */
            for (int32_t f = 9; f < MAX_THINGS; f++) {
                if (doom_st.things[f].kind == TH_NONE) {
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
            }
            th->timer = 34;
        } else if (th->kind == TH_ZOMBIE && dist < 300 && doom_st.health > 0) {
            if ((doom_rand() & 3u) == 0u) hurt_player(4);
            th->timer = 28;
        }
    }
}

/* ---------- 2D Drawing: Weapon Viewmodel, Status Bar, Mugshot & Automap (DOOM_D) ---------- */

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
static void draw_big_num(int32_t x, int32_t y, int32_t val, int pct) {
    uint8_t buf[5];
    int32_t n = 0;
    if (val < 0) val = 0;
    if (val > 999) val = 999;
    do {
        uint32_t q = doom_udiv((uint32_t)val, 10u);
        buf[n++] = (uint8_t)((uint32_t)val - q * 10u);
        val = (int32_t)q;
    } while (val > 0 && n < 3);

    int32_t cx = x - (n + (pct ? 1 : 0)) * 10;
    for (int32_t i = n - 1; i >= 0; i--) {
        uint32_t g = hud_digits[buf[i]];
        for (int32_t r = 0; r < 5; r++) {
            uint32_t row = (g >> ((4 - r) * 4)) & 15u;
            for (int32_t col = 0; col < 4; col++) {
                if (row & (8u >> col)) {
                    fb_rect(cx + col * 2, y + r * 3, 2, 3, (r == 0) ? 112u : 114u);
                }
            }
        }
        cx += 10;
    }
    if (pct) {
        uint32_t g = hud_digits[10];
        for (int32_t r = 0; r < 5; r++) {
            uint32_t row = (g >> ((4 - r) * 4)) & 15u;
            for (int32_t col = 0; col < 4; col++)
                if (row & (8u >> col)) fb_rect(cx + col * 2, y + r * 3, 2, 3, 114u);
        }
    }
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

/* First-person weapon viewmodel (Pistol / Shotgun / Chaingun) + muzzle flash! */
__attribute__((section(".doom.hud")))
static void draw_weapon(void) {
    int32_t bob_x = (d_sin(doom_st.tic << 5) >> 12);
    int32_t bob_y = (d_cos(doom_st.tic << 6) >> 13) + (int32_t)doom_st.recoil * 2;
    int32_t cx = D_HALF_W + bob_x;
    int32_t by = D_VIEWH + bob_y;

    /* Muzzle flash */
    if (doom_st.flash_tics) {
        int32_t fy = by - (doom_st.weapon == WPN_SHOTGUN ? 68 : 60);
        fb_rect(cx - 18, fy - 12, 36, 18, 128u);
        fb_rect(cx - 10, fy - 20, 20, 26, 129u);
        fb_rect(cx - 5,  fy - 14, 10, 14, 0u);
    }

    if (doom_st.weapon == WPN_PISTOL) {
        /* Classic Doom Beretta pistol + gloved hand */
        fb_rect(cx - 7,  by - 58, 14, 24, 5u);
        fb_rect(cx - 5,  by - 56, 10, 20, 2u);
        fb_rect(cx - 3,  by - 62,  6,  6, 10u); /* sight */
        fb_rect(cx - 11, by - 36, 22, 36, 162u); /* fist */
        fb_rect(cx - 13, by - 22, 26, 22, 165u);
    } else if (doom_st.weapon == WPN_SHOTGUN) {
        /* Iconic Doom Pump Shotgun */
        fb_rect(cx - 10, by - 66, 20, 32, 6u);
        fb_rect(cx - 6,  by - 68, 12, 26, 3u);
        fb_rect(cx - 3,  by - 70,  6,  4, 128u); /* brass bead */
        fb_rect(cx - 14, by - 36, 28, 36, 52u);  /* wood forend */
        fb_rect(cx - 10, by - 28, 20, 28, 55u);
    } else {
        /* Chaingun rotating barrels */
        int32_t spin = (doom_st.tic & 1u) ? 2 : 0;
        fb_rect(cx - 22, by - 56, 44, 56, 8u);
        for (int32_t b = -16 + spin; b <= 14; b += 8) {
            fb_rect(cx + b, by - 62, 5, 42, 4u);
            fb_rect(cx + b + 1, by - 62, 2, 42, 1u);
        }
        fb_rect(cx - 24, by - 30, 48, 8, 130u); /* brass ammo belt */
    }
}

/* Classic Doom Status Bar (y = 200..239) with animated DOOMGUY mugshot! */
__attribute__((section(".doom.hud")))
static void draw_status_bar(void) {
    /* Embossed grey stone/steel base */
    fb_rect(0, 200, D_W, 40, 10u);
    fb_rect(0, 200, D_W, 2, 4u);
    fb_rect(0, 238, D_W, 2, 14u);
    /* Panel dividers */
    static const int16_t divs[4] = { 62, 136, 184, 256 };
    for (int32_t i = 0; i < 4; i++) {
        fb_rect(divs[i], 202, 2, 36, 14u);
        fb_rect(divs[i] + 2, 202, 1, 36, 6u);
    }

    draw_big_num(56, 206, doom_st.ammo, 0);
    draw_micro_str(18, 229, "AMMO", 3u);

    draw_big_num(130, 206, doom_st.health, 1);
    draw_micro_str(86, 229, "HEALTH", 3u);

    draw_big_num(250, 206, doom_st.armor, 1);
    draw_micro_str(206, 229, "ARMOR", 3u);

    draw_big_num(312, 206, doom_st.kills, 0);
    draw_micro_str(274, 229, "KILLS", 3u);

    /* Weapon slot indicator dots (1/2/3) */
    for (uint32_t w = 0; w < 3u; w++) {
        fb_rect(266 + (int32_t)w * 10, 208, 6, 8, (doom_st.weapon == w) ? 128u : 13u);
    }

    /* Animated DOOMGUY Mugshot (centered x = 144..176, y = 203..236) */
    int32_t fx = 146, fy = 204;
    fb_rect(fx - 2, fy - 1, 32, 34, 14u);
    fb_rect(fx, fy, 28, 32, 12u);
    /* Hair */
    fb_rect(fx + 5, fy + 2, 18, 6, 52u);
    fb_rect(fx + 4, fy + 5, 20, 4, 54u);
    /* Face skin */
    fb_rect(fx + 6, fy + 8, 16, 18, 162u);
    fb_rect(fx + 8, fy + 24, 12, 4, 164u); /* chin */
    /* Bloodied bruise when low health or hurt */
    if (doom_st.health < 60 || doom_st.pain_tics) {
        fb_rect(fx + 7, fy + 16, 4, 4, 114u);
        fb_rect(fx + 18, fy + 11, 3, 5, 112u);
    }
    /* Glancing eyes */
    int32_t look = (doom_st.face_expr == 0) ? -1 : (doom_st.face_expr == 2) ? 1 : 0;
    fb_rect(fx + 8,  fy + 13, 4, 3, 1u);
    fb_rect(fx + 16, fy + 13, 4, 3, 1u);
    fb_rect(fx + 9 + look,  fy + 14, 2, 2, 15u);
    fb_rect(fx + 17 + look, fy + 14, 2, 2, 15u);
    /* Expression mouth (grim / wicked grin / ouch) */
    if (doom_st.face_expr == 3) {
        fb_rect(fx + 9, fy + 21, 10, 3, 1u); /* grinning teeth! */
    } else if (doom_st.face_expr == 4) {
        fb_rect(fx + 11, fy + 20, 6, 5, 114u); /* ouch! */
    } else {
        fb_rect(fx + 10, fy + 22, 8, 2, 66u); /* stern scowl */
    }

    /* Victory banner if EXIT switch activated */
    if (doom_st.won) {
        fb_rect(96, 82, 128, 32, 14u);
        fb_rect(98, 84, 124, 28, 118u);
        draw_micro_str(122, 92, "LEVEL CLEAR", 128u);
        draw_micro_str(116, 102, "PRESS QS PLAY", 0u);
    }
}

/* Bresenham line for Doom Automap */
__attribute__((section(".doom.hud")))
static void am_line(int32_t x0, int32_t y0, int32_t x1, int32_t y1, uint8_t c) {
    int32_t dx = x1 - x0, dy = y1 - y0;
    int32_t sx = dx >= 0 ? 1 : -1, sy = dy >= 0 ? 1 : -1;
    dx = dx < 0 ? -dx : dx;
    dy = dy < 0 ? -dy : dy;
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

/* Classic Doom Vector Automap (toggled by DISP key 0x07) */
__attribute__((section(".doom.hud")))
static void draw_automap(void) {
    fb_rect(0, 0, D_W, D_VIEWH, 15u); /* dark slate map backdrop */
    int32_t cx = doom_st.px >> 16, cy = doom_st.py >> 16;
    for (uint32_t i = 0; i < 66u; i++) {
        const struct d_line *ln = &doom_map_lines[i];
        int32_t x1 = D_HALF_W + ((ln->x1 - cx) >> 2);
        int32_t y1 = D_HALF_H - ((ln->y1 - cy) >> 2);
        int32_t x2 = D_HALF_W + ((ln->x2 - cx) >> 2);
        int32_t y2 = D_HALF_H - ((ln->y2 - cy) >> 2);
        uint8_t col = (ln->flags & LINE_EXIT) ? 96u
                    : (ln->flags & LINE_DOOR) ? 144u
                    : (ln->back < 0) ? 112u : 130u;
        am_line(x1, y1, x2, y2, col);
    }
    /* Enemy blips */
    for (int32_t i = 0; i < MAX_THINGS; i++) {
        const struct d_thing *th = &doom_st.things[i];
        if (th->kind == TH_NONE || th->state == 3) continue;
        int32_t tx = D_HALF_W + (((th->x >> 16) - cx) >> 2);
        int32_t ty = D_HALF_H - (((th->y >> 16) - cy) >> 2);
        fb_rect(tx - 1, ty - 1, 3, 3, (th->kind <= TH_DEMON) ? 112u : 96u);
    }
    /* White player direction arrow */
    int32_t fx = (d_cos(doom_st.angle) * 10) >> 14;
    int32_t fy = (d_sin(doom_st.angle) * 10) >> 14;
    am_line(D_HALF_W - (fx >> 1), D_HALF_H + (fy >> 1), D_HALF_W + fx, D_HALF_H - fy, 0u);
    fb_rect(D_HALF_W - 1, D_HALF_H - 1, 3, 3, 0u);
    draw_micro_str(8, 8, "E HANGAR", 128u);
}

/* Render complete 320x240 frame and 2x2 upscale directly into 640x480 ARGB_FB */
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
    hw_dcache_sync();
    stock_flush_rect(0, 0, 640, 480);
    stock_argb_commit();
}

/* ---------- M6M Screen Vtable Handler (Screen 33 = 0x21) & Menu Launcher ---------- */

uint32_t doom_screen(uint32_t msg, uint32_t arg) {
    if (msg == 4u) {
        if (arg == 0u || arg == 1u) {
            /* Screen entry: init DOOM, wipe 8bpp OSD plane so no camera icons float over game */
            doom_ensure_init();
            stock_osd_clear();
            stock_osd_commit();
            doom_present();
        } else if (arg == 3u) {
            /* Periodic refresh / timer tick */
            doom_ensure_init();
            doom_step_sim();
            doom_present();
        }
        return 0u;
    }

    if (msg == 3u) {
        /* Screen close / teardown */
        stock_argb_clear();
        stock_argb_commit();
        stock_osd_clear();
        stock_osd_commit();
        return 0u;
    }

    if (msg == 5u) {
        doom_ensure_init();
        /* Exit cleanly back to Live View (SCREEN_LV = 0x15) on Menu (0x06) or Play (0x0C) */
        if (arg == 0x06u || arg == 0x0Cu) {
            stock_argb_clear();
            stock_argb_commit();
            stock_osd_clear();
            stock_osd_commit();
            stock_screen_goto(SCREEN_LV);
            return 0u;
        }

        int32_t ca = d_cos(doom_st.angle);
        int32_t sa = d_sin(doom_st.angle);

        switch (arg) {
        case 0x02u: /* Up: Move forward */
            move_player(ca << 6, sa << 6);
            doom_st.pvx = ca << 4;
            doom_st.pvy = sa << 4;
            break;
        case 0x03u: /* Down: Move backward */
            move_player(-(ca << 6), -(sa << 6));
            doom_st.pvx = -(ca << 4);
            doom_st.pvy = -(sa << 4);
            break;
        case 0x04u: /* Left: Turn left */
            doom_st.angle = (doom_st.angle + 32u) & ANG_MASK;
            break;
        case 0x05u: /* Right: Turn right */
            doom_st.angle = (doom_st.angle - 32u) & ANG_MASK;
            break;
        case 0x01u: /* Ok: Fire! */
        case 0xFEu: /* Shutter: Fire! */
            doom_fire();
            break;
        case 0x08u: /* AEL: Use / Open Door / Exit Switch */
            doom_use();
            break;
        case 0x07u: /* DISP: Toggle Automap */
            doom_st.automap ^= 1u;
            break;
        case 0x09u: /* QS: Restart level */
            doom_reset_level();
            break;
        case 0x20u: /* DialLeft: Previous weapon */
            doom_st.weapon = (uint8_t)(doom_st.weapon ? doom_st.weapon - 1u : 2u);
            break;
        case 0x40u: /* DialRight: Next weapon */
        case 0x0Au: /* Mode: Cycle weapon */
            doom_st.weapon = (uint8_t)(doom_st.weapon >= 2u ? 0u : doom_st.weapon + 1u);
            break;
        default:
            break;
        }

        doom_step_sim();
        doom_present();
    }
    return 0u;
}

/* Menu row extra handler (w7 in Setup Page 4): opens SCREEN_DOOM (33 = 0x21) */
uint32_t doom_open(void) {
    /* Reference doom_screen so --gc-sections retains it iff DOOM is enabled in Setup Page 4 */
    volatile uint32_t keep = (uint32_t)(uintptr_t)doom_screen;
    (void)keep;
    stock_screen_post(1u, SCREEN_DOOM);
    return 0u;
}
