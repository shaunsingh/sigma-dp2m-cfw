/* SPDX-License-Identifier: GPL-2.0-or-later */
/* DP2M RGBA4444 Bayer-Dithered Video Blitter + 3-Mode Tech Demo HUD & HW Explorer */
#include <stdint.h>
#include <stdio.h>
#include <stdlib.h>
#include <string.h>

#include "config.h"
#include "doomstat.h"
#include "doomtype.h"
#include "i_system.h"
#include "i_video.h"
#include "m_argv.h"
#include "v_video.h"
#include "z_zone.h"
#include "doomgeneric.h"
#include "dp2m_doom.h"

extern void I_InitInput(void);
extern void I_GetEvent(void);
extern uint8_t __image_end[];
extern uint8_t __bss_end[];

/* Globals expected by doomgeneric core */
byte *I_VideoBuffer = NULL;
boolean screenvisible = true;
boolean screensaver_mode = false;
int usegamma = 0;
int usemouse = 0;
float mouse_acceleration = 2.0f;
int mouse_threshold = 10;
int screen_width = SCREENWIDTH;
int screen_height = SCREENHEIGHT;
int screen_bpp = 8;
int fullscreen = 1;
int aspect_ratio_correct = 1;
int show_diskicon = 0;
int diskicon_readbytes = 0;
char *video_driver = "dp2m-rgba4444";

/* 2x2 Bayer-dithered big-endian RGBA4444 pair LUTs (2 KB total, fits in L1 D$) */
static uint32_t g_dither_row0[256] __attribute__((aligned(32)));
static uint32_t g_dither_row1[256] __attribute__((aligned(32)));
static uint8_t  g_pal_rgb[256][3];

#define LCD_W 640
#define LCD_H 480

/* Oxocarbon palette rounded to big-endian RGBA4444: (R<<12)|(G<<8)|(B<<4)|A */
#define COL_BG_DARK   0x111Fu /* #161616 */
#define COL_BG_BAR    0x222Fu /* #262626 */
#define COL_BG_PILL   0x333Fu /* #393939 */
#define COL_PANEL_TR  0x111Du /* #161616, semi-transparent explorer backdrop (A=0xD) */
#define COL_HDR_TR    0x333Eu /* #393939 */
#define COL_GRID      0x555Fu /* #525252 */
#define COL_ACCENT    0xF7BFu /* #ff7eb6 */
#define COL_PURPLE    0xB9FFu /* #be95ff */
#define COL_BLUE      0x7AFFu /* #78a9ff */
#define COL_CYAN      0x4DDFu /* #3ddbd9 */
#define COL_GREEN     0x4B6Fu /* #42be65 */
#define COL_YELLOW    0xEB2Fu /* #f1c21b */
#define COL_RED_LIT   0xE59Fu /* #ee5396 */
#define COL_WHITE     0xFFFFu /* #ffffff */
#define COL_LIGHT     0xDDEFu /* #dde1e6 */
#define COL_MUTED     0x888Fu /* #8d8d8d */

/* Crisp 5x7 full printable ASCII font (0x20..0x7E, 5 columns, LSB = top row) */
static const uint8_t font5x7[95][5] = {
    {0x00,0x00,0x00,0x00,0x00}, /* ' ' */
    {0x00,0x00,0x5F,0x00,0x00}, /* '!' */
    {0x00,0x07,0x00,0x07,0x00}, /* '"' */
    {0x14,0x7F,0x14,0x7F,0x14}, /* '#' */
    {0x24,0x2A,0x7F,0x2A,0x12}, /* '$' */
    {0x23,0x13,0x08,0x64,0x62}, /* '%' */
    {0x36,0x49,0x55,0x22,0x50}, /* '&' */
    {0x00,0x05,0x03,0x00,0x00}, /* ''' */
    {0x00,0x1C,0x22,0x41,0x00}, /* '(' */
    {0x00,0x41,0x22,0x1C,0x00}, /* ')' */
    {0x14,0x08,0x3E,0x08,0x14}, /* '*' */
    {0x08,0x08,0x3E,0x08,0x08}, /* '+' */
    {0x00,0x50,0x30,0x00,0x00}, /* ',' */
    {0x08,0x08,0x08,0x08,0x08}, /* '-' */
    {0x00,0x60,0x60,0x00,0x00}, /* '.' */
    {0x20,0x10,0x08,0x04,0x02}, /* '/' */
    {0x3E,0x51,0x49,0x45,0x3E}, /* '0' */
    {0x00,0x42,0x7F,0x40,0x00}, /* '1' */
    {0x42,0x61,0x51,0x49,0x46}, /* '2' */
    {0x21,0x41,0x45,0x4B,0x31}, /* '3' */
    {0x18,0x14,0x12,0x7F,0x10}, /* '4' */
    {0x27,0x45,0x45,0x45,0x39}, /* '5' */
    {0x3C,0x4A,0x49,0x49,0x30}, /* '6' */
    {0x01,0x71,0x09,0x05,0x03}, /* '7' */
    {0x36,0x49,0x49,0x49,0x36}, /* '8' */
    {0x06,0x49,0x49,0x29,0x1E}, /* '9' */
    {0x00,0x36,0x36,0x00,0x00}, /* ':' */
    {0x00,0x56,0x36,0x00,0x00}, /* ';' */
    {0x08,0x14,0x22,0x41,0x00}, /* '<' */
    {0x14,0x14,0x14,0x14,0x14}, /* '=' */
    {0x00,0x41,0x22,0x14,0x08}, /* '>' */
    {0x02,0x01,0x51,0x09,0x06}, /* '?' */
    {0x32,0x49,0x79,0x41,0x3E}, /* '@' */
    {0x7E,0x11,0x11,0x11,0x7E}, /* 'A' */
    {0x7F,0x49,0x49,0x49,0x36}, /* 'B' */
    {0x3E,0x41,0x41,0x41,0x22}, /* 'C' */
    {0x7F,0x41,0x41,0x22,0x1C}, /* 'D' */
    {0x7F,0x49,0x49,0x49,0x41}, /* 'E' */
    {0x7F,0x09,0x09,0x09,0x01}, /* 'F' */
    {0x3E,0x41,0x49,0x49,0x7A}, /* 'G' */
    {0x7F,0x08,0x08,0x08,0x7F}, /* 'H' */
    {0x00,0x41,0x7F,0x41,0x00}, /* 'I' */
    {0x20,0x40,0x41,0x3F,0x01}, /* 'J' */
    {0x7F,0x08,0x14,0x22,0x41}, /* 'K' */
    {0x7F,0x40,0x40,0x40,0x40}, /* 'L' */
    {0x7F,0x02,0x0C,0x02,0x7F}, /* 'M' */
    {0x7F,0x04,0x08,0x10,0x7F}, /* 'N' */
    {0x3E,0x41,0x41,0x41,0x3E}, /* 'O' */
    {0x7F,0x09,0x09,0x09,0x06}, /* 'P' */
    {0x3E,0x41,0x51,0x21,0x5E}, /* 'Q' */
    {0x7F,0x09,0x19,0x29,0x46}, /* 'R' */
    {0x46,0x49,0x49,0x49,0x31}, /* 'S' */
    {0x01,0x01,0x7F,0x01,0x01}, /* 'T' */
    {0x3F,0x40,0x40,0x40,0x3F}, /* 'U' */
    {0x1F,0x20,0x40,0x20,0x1F}, /* 'V' */
    {0x3F,0x40,0x38,0x40,0x3F}, /* 'W' */
    {0x63,0x14,0x08,0x14,0x63}, /* 'X' */
    {0x07,0x08,0x70,0x08,0x07}, /* 'Y' */
    {0x61,0x51,0x49,0x45,0x43}, /* 'Z' */
    {0x00,0x7F,0x41,0x41,0x00}, /* '[' */
    {0x02,0x04,0x08,0x10,0x20}, /* '\' */
    {0x00,0x41,0x41,0x7F,0x00}, /* ']' */
    {0x04,0x02,0x01,0x02,0x04}, /* '^' */
    {0x40,0x40,0x40,0x40,0x40}, /* '_' */
    {0x00,0x01,0x02,0x04,0x00}, /* '`' */
    {0x20,0x54,0x54,0x54,0x78}, /* 'a' */
    {0x7F,0x48,0x44,0x44,0x38}, /* 'b' */
    {0x38,0x44,0x44,0x44,0x20}, /* 'c' */
    {0x38,0x44,0x44,0x48,0x7F}, /* 'd' */
    {0x38,0x54,0x54,0x54,0x18}, /* 'e' */
    {0x08,0x7E,0x09,0x01,0x02}, /* 'f' */
    {0x0C,0x52,0x52,0x52,0x3E}, /* 'g' */
    {0x7F,0x08,0x04,0x04,0x78}, /* 'h' */
    {0x00,0x44,0x7D,0x40,0x00}, /* 'i' */
    {0x20,0x40,0x44,0x3D,0x00}, /* 'j' */
    {0x7F,0x10,0x28,0x44,0x00}, /* 'k' */
    {0x00,0x41,0x7F,0x40,0x00}, /* 'l' */
    {0x7C,0x04,0x18,0x04,0x78}, /* 'm' */
    {0x7C,0x08,0x04,0x04,0x78}, /* 'n' */
    {0x38,0x44,0x44,0x44,0x38}, /* 'o' */
    {0x7C,0x14,0x14,0x14,0x08}, /* 'p' */
    {0x08,0x14,0x14,0x18,0x7C}, /* 'q' */
    {0x7C,0x08,0x04,0x04,0x08}, /* 'r' */
    {0x48,0x54,0x54,0x54,0x20}, /* 's' */
    {0x04,0x3F,0x44,0x40,0x20}, /* 't' */
    {0x3C,0x40,0x40,0x20,0x7C}, /* 'u' */
    {0x1C,0x20,0x40,0x20,0x1C}, /* 'v' */
    {0x3C,0x40,0x30,0x40,0x3C}, /* 'w' */
    {0x44,0x28,0x10,0x28,0x44}, /* 'x' */
    {0x0C,0x50,0x50,0x50,0x3C}, /* 'y' */
    {0x44,0x64,0x54,0x4C,0x44}, /* 'z' */
    {0x00,0x08,0x36,0x41,0x00}, /* '{' */
    {0x00,0x00,0x7F,0x00,0x00}, /* '|' */
    {0x00,0x41,0x36,0x08,0x00}, /* '}' */
    {0x10,0x08,0x08,0x10,0x08}, /* '~' */
};

static inline uint32_t rd_reg32(uint32_t va) {
    return *(volatile const uint32_t *)(uintptr_t)va;
}

static inline uint8_t rd_reg8(uint32_t va) {
    return *(volatile const uint8_t *)(uintptr_t)va;
}

/* Matches stock_ui_screen_id (0xA004333C): top non-zero slot in 0xA07DEB20..0xA07DEB27 */
static uint8_t rd_active_screen_id(void) {
    for (int i = 7; i >= 0; i--) {
        uint8_t s = rd_reg8(0xA07DEB20u + (uint32_t)i);
        if (s != 0u) return s;
    }
    return 0u;
}

static inline uint32_t rd_cpsr(void) {
    uint32_t v = 0u;
    __asm__ volatile("mrs %0, cpsr" : "=r"(v));
    return v;
}

static inline uint32_t rd_sp(void) {
    uint32_t v = 0u;
    __asm__ volatile("mov %0, sp" : "=r"(v));
    return v;
}

/* ---------- Fast 32-bit Word Rect & 5x7 Text Primitives ---------- */

static void fb_fill_rect(uint16_t *fb, int x, int y, int w, int h, uint16_t col) {
    if (x < 0) { w += x; x = 0; }
    if (y < 0) { h += y; y = 0; }
    if (x + w > LCD_W) w = LCD_W - x;
    if (y + h > LCD_H) h = LCD_H - y;
    if (w <= 0 || h <= 0) return;

    uint32_t pair = ((uint32_t)col << 16) | col;
    for (int r = 0; r < h; r++) {
        uint16_t *row = fb + (y + r) * LCD_W + x;
        int rem = w;
        if ((uintptr_t)row & 2u) { *row++ = col; rem--; }
        uint32_t *w32 = (uint32_t *)(void *)row;
        while (rem >= 8) {
            w32[0] = pair; w32[1] = pair; w32[2] = pair; w32[3] = pair;
            w32 += 4;
            rem -= 8;
        }
        while (rem >= 2) { *w32++ = pair; rem -= 2; }
        if (rem) *(uint16_t *)(void *)w32 = col;
    }
}

static void fb_draw_box(uint16_t *fb, int x, int y, int w, int h, uint16_t border, uint16_t fill) {
    fb_fill_rect(fb, x, y, w, h, fill);
    fb_fill_rect(fb, x, y, w, 1, border);
    fb_fill_rect(fb, x, y + h - 1, w, 1, border);
    fb_fill_rect(fb, x, y, 1, h, border);
    fb_fill_rect(fb, x + w - 1, y, 1, h, border);
}

static int fb_draw_str(uint16_t *fb, int x, int y, const char *s, uint16_t col) {
    for (; *s; s++, x += 6) {
        if (x + 5 > LCD_W) break;
        if (x < 0) continue;
        unsigned int c = (unsigned char)*s;
        if (c < 0x20u || c > 0x7Eu) c = '?';
        const uint8_t *glyph = font5x7[c - 0x20u];
        for (int col_i = 0; col_i < 5; col_i++) {
            uint8_t bits = glyph[col_i];
            if (!bits) continue;
            for (int row_i = 0; row_i < 7; row_i++) {
                if ((bits & (1u << row_i)) && (unsigned)(y + row_i) < (unsigned)LCD_H)
                    fb[(y + row_i) * LCD_W + x + col_i] = col;
            }
        }
    }
    return x;
}

static int fb_draw_pill(uint16_t *fb, int x, int y, const char *label, int active, uint16_t on_bg) {
    int len = (int)strlen(label);
    int pw = len * 6 + 5;
    fb_fill_rect(fb, x, y - 1, pw, 9, active ? on_bg : COL_BG_PILL);
    fb_draw_str(fb, x + 3, y, label, active ? COL_BG_DARK : COL_MUTED);
    return x + pw + 2;
}

/* ---------- Boot Progress Splash Screen ---------- */

void dp2m_draw_boot_progress(const char *stage, uint32_t done, uint32_t total) {
    if (!g_dp2m_api || !g_dp2m_api->argb_fb || !g_dp2m_api->present_argb) return;
    uint16_t *fb = g_dp2m_api->argb_fb;
    fb_fill_rect(fb, 0, 0, LCD_W, LCD_H, COL_BG_DARK);
    fb_draw_box(fb, 60, 168, 520, 144, COL_ACCENT, COL_BG_BAR);
    fb_fill_rect(fb, 61, 169, 518, 22, COL_HDR_TR);

    fb_draw_str(fb, 76, 177, "SIGMA DP2 MERRILL // DOOM.BIN STANDALONE ENGINE", COL_ACCENT);

    char line[96];
    snprintf(line, sizeof(line), "EXEC: 0xA1CC0000..0x%08X   BSS: 0x%08X   SP: 0x%08X",
             (unsigned)(uintptr_t)__image_end,
             (unsigned)(uintptr_t)__bss_end,
             (unsigned)rd_sp());
    fb_draw_str(fb, 76, 204, line, COL_BLUE);

    snprintf(line, sizeof(line), "%s (%u / %u KB)",
             stage ? stage : "INITIALIZING",
             (unsigned)(done >> 10),
             (unsigned)(total >> 10));
    fb_draw_str(fb, 76, 224, line, COL_WHITE);

    /* Progress bar */
    fb_draw_box(fb, 76, 244, 488, 18, COL_GRID, COL_BG_DARK);
    if (total > 0u) {
        uint32_t fill_w = (done >= total) ? 484u : ((done >> 8) * 484u) / ((total >> 8) ? (total >> 8) : 1u);
        if (fill_w > 484u) fill_w = 484u;
        fb_fill_rect(fb, 78, 246, (int)fill_w, 14, COL_ACCENT);
    }

    fb_draw_str(fb, 76, 276, "ARM946E-S BIG-ENDIAN @ 216 MHZ  |  2X2 BAYER RGBA4444  |  MPU R2 EXEC", COL_MUTED);
    g_dp2m_api->present_argb();
}

/* ---------- 2x2 Bayer Ordered-Dithered RGBA4444 Palette & Blitters ---------- */

static inline uint16_t quant_rgba4444(uint32_t r, uint32_t g, uint32_t b, uint32_t bias) {
    uint32_t rq = r * 15u + bias; rq = (rq + (rq >> 8)) >> 8;
    uint32_t gq = g * 15u + bias; gq = (gq + (gq >> 8)) >> 8;
    uint32_t bq = b * 15u + bias; bq = (bq + (bq >> 8)) >> 8;
    return (uint16_t)((rq << 12) | (gq << 8) | (bq << 4) | 0xFu);
}

void I_SetPalette(byte *palette) {
    int g = (usegamma >= 0 && usegamma < 5) ? usegamma : 0;
    for (int i = 0; i < 256; i++) {
        uint32_t r = gammatable[g][palette[0]];
        uint32_t gr = gammatable[g][palette[1]];
        uint32_t b = gammatable[g][palette[2]];
        palette += 3;
        g_pal_rgb[i][0] = (uint8_t)r;
        g_pal_rgb[i][1] = (uint8_t)gr;
        g_pal_rgb[i][2] = (uint8_t)b;

        /* 2x2 Bayer matrix [[0, 2], [3, 1]] -> unbiased offsets {32, 160, 224, 96} */
        uint16_t p00 = quant_rgba4444(r, gr, b, 32u);
        uint16_t p01 = quant_rgba4444(r, gr, b, 160u);
        uint16_t p10 = quant_rgba4444(r, gr, b, 224u);
        uint16_t p11 = quant_rgba4444(r, gr, b, 96u);
        g_dither_row0[i] = ((uint32_t)p00 << 16) | p01;
        g_dither_row1[i] = ((uint32_t)p10 << 16) | p11;
    }
}

int I_GetPaletteIndex(int r, int g, int b) {
    int best = 0;
    uint32_t best_dist = 0xFFFFFFFFu;
    for (int i = 0; i < 256; i++) {
        int dr = r - (int)g_pal_rgb[i][0];
        int dg = g - (int)g_pal_rgb[i][1];
        int db = b - (int)g_pal_rgb[i][2];
        uint32_t d = (uint32_t)(dr * dr + dg * dg + db * db);
        if (d < best_dist) { best_dist = d; best = i; }
    }
    return best;
}

static inline void blit_scanline_pair(const uint8_t *src, uint32_t *dst, const uint32_t *lut) {
    for (int x = 0; x < 320; x += 8) {
        uint32_t p0 = lut[src[x + 0]];
        uint32_t p1 = lut[src[x + 1]];
        uint32_t p2 = lut[src[x + 2]];
        uint32_t p3 = lut[src[x + 3]];
        uint32_t p4 = lut[src[x + 4]];
        uint32_t p5 = lut[src[x + 5]];
        uint32_t p6 = lut[src[x + 6]];
        uint32_t p7 = lut[src[x + 7]];
        dst[0] = p0; dst[1] = p1; dst[2] = p2; dst[3] = p3;
        dst[4] = p4; dst[5] = p5; dst[6] = p6; dst[7] = p7;
        dst += 8;
    }
}

/* 320x200 -> 640x400 centered at y = 40..439 */
static void blit_640x400_centered(const uint8_t *src, uint16_t *fb) {
    const uint32_t *lut0 = g_dither_row0;
    const uint32_t *lut1 = g_dither_row1;
    uint32_t *dst = (uint32_t *)(void *)(fb + 40 * LCD_W);
    for (int y = 0; y < 200; y++) {
        blit_scanline_pair(src, dst, lut0);
        blit_scanline_pair(src, dst + 320, lut1);
        src += 320;
        dst += 640;
    }
}

/* 320x200 -> 640x480 authentic 4:3 CRT aspect stretch (5 -> 12 row cadence: 2,3,2,3,2) */
static void blit_640x480_crt43(const uint8_t *src, uint16_t *fb) {
    static const uint8_t row_rep[5] = { 2, 3, 2, 3, 2 };
    const uint32_t *lut0 = g_dither_row0;
    const uint32_t *lut1 = g_dither_row1;
    uint32_t *dst = (uint32_t *)(void *)fb;
    int out_y = 0;
    for (int y = 0; y < 200; y++) {
        int rep = row_rep[y % 5];
        for (int r = 0; r < rep; r++, out_y++) {
            blit_scanline_pair(src, dst, (out_y & 1) ? lut1 : lut0);
            dst += 320;
        }
        src += 320;
    }
}

/* ---------- Tech Demo Dashboard & Live Hardware Explorer Overlays ---------- */

static void draw_dashboard_bars(uint16_t *fb) {
    uint32_t pc_here = (uint32_t)(uintptr_t)&draw_dashboard_bars;
    uint32_t sp_now  = rd_sp();
    uint32_t cpsr    = rd_cpsr();
    uint32_t tick_hi = rd_reg32(0xA07F3788u);
    uint32_t tick_lo = rd_reg32(0xA07F378Cu);

    /* Top 40px bar (y = 0..39) */
    fb_fill_rect(fb, 0, 0, LCD_W, 39, COL_BG_BAR);
    fb_fill_rect(fb, 0, 39, LCD_W, 1, COL_ACCENT);

    char line[112];
    fb_draw_str(fb, 6, 4, "SIGMA DP2M // ARM946E-S BE", COL_ACCENT);
    snprintf(line, sizeof(line), "DOOM.BIN @ A1CC0000 (%uK)",
             (unsigned)(((uintptr_t)__image_end - 0xA1CC0000u) >> 10));
    fb_draw_str(fb, 176, 4, line, COL_BLUE);
    snprintf(line, sizeof(line), "FPS:%2u (%2uMS) TIC:%5u",
             (unsigned)g_doom_tel.fps, (unsigned)g_doom_tel.frame_ms, (unsigned)gametic);
    fb_draw_str(fb, 492, 4, line, COL_GREEN);

    snprintf(line, sizeof(line), "PC:%08X SP:%08X CPSR:%08X  TICK:%08X/%08X  PSAVE:%08X",
             (unsigned)pc_here, (unsigned)sp_now, (unsigned)cpsr,
             (unsigned)tick_hi, (unsigned)tick_lo, (unsigned)rd_reg32(0xA07DD73Cu));
    fb_draw_str(fb, 6, 16, line, COL_LIGHT);

    snprintf(line, sizeof(line), "WAD:%-12s %4uK [%s] RD:%4u (%5uK)  ZONE:%4uK HEAP:%4uK (#%2u)",
             g_doom_tel.wad_name[0] ? g_doom_tel.wad_name : "DOOM1.WAD",
             (unsigned)(g_doom_tel.wad_size_bytes >> 10),
             g_doom_tel.wad_preloaded ? "SDRAM" : "MDF  ",
             (unsigned)g_doom_tel.wad_read_calls,
             (unsigned)(g_doom_tel.wad_read_bytes >> 10),
             (unsigned)(g_doom_tel.zone_bytes >> 10),
             (unsigned)(g_doom_tel.heap_bytes >> 10),
             (unsigned)g_doom_tel.heap_blocks);
    fb_draw_str(fb, 6, 28, line, COL_MUTED);

    /* Bottom 40px bar (y = 440..479) */
    fb_fill_rect(fb, 0, 440, LCD_W, 1, COL_ACCENT);
    fb_fill_rect(fb, 0, 441, LCD_W, 39, COL_BG_BAR);

    uint32_t m = g_doom_tel.inp.held_mask;
    int x = 6;
    x = fb_draw_pill(fb, x, 444, "UP",   (m & DP2M_BTN_UP) != 0,      COL_GREEN);
    x = fb_draw_pill(fb, x, 444, "DN",   (m & DP2M_BTN_DOWN) != 0,    COL_GREEN);
    x = fb_draw_pill(fb, x, 444, "LF",   (m & DP2M_BTN_LEFT) != 0,    COL_GREEN);
    x = fb_draw_pill(fb, x, 444, "RT",   (m & DP2M_BTN_RIGHT) != 0,   COL_GREEN);
    x = fb_draw_pill(fb, x, 444, "SHUT", (m & DP2M_BTN_SHUTTER) != 0, COL_RED_LIT);
    x = fb_draw_pill(fb, x, 444, "OK",   (m & DP2M_BTN_OK) != 0,      COL_GREEN);
    x = fb_draw_pill(fb, x, 444, "AEL",  (m & DP2M_BTN_AEL) != 0,     COL_GREEN);
    x = fb_draw_pill(fb, x, 444, "QS",   (m & DP2M_BTN_QS) != 0,      COL_PURPLE);
    x = fb_draw_pill(fb, x, 444, "DISP", (m & DP2M_BTN_DISP) != 0,    COL_CYAN);
    x = fb_draw_pill(fb, x, 444, "MODE", (m & DP2M_BTN_MODE) != 0,    COL_CYAN);

    snprintf(line, sizeof(line), "K0:%08X SW:%08X DL:%u TILT:%+3d/%+3d",
             (unsigned)g_doom_tel.inp.raw_key0,
             (unsigned)g_doom_tel.inp.raw_sw28,
             (unsigned)g_doom_tel.inp.raw_dial,
             (int)g_doom_tel.inp.accel_x,
             (int)g_doom_tel.inp.accel_y);
    fb_draw_str(fb, x + 4, 444, line, COL_LIGHT);

    x = fb_draw_pill(fb, 6, 456, g_doom_tel.tilt_strafe ? "TILT:ON" : "TILT:OFF",
                     (int)g_doom_tel.tilt_strafe, COL_PURPLE);
    fb_draw_str(fb, x + 4, 456,
                "SHUT:FIRE AEL:USE DIAL:TURN QS+LR:STRF QS+DL:WPN QS+AEL:TILT MODE:HUD PLAY:EXIT",
                COL_MUTED);

    /* Bottom row: 64-frame real-time frame-time sparkline */
    fb_draw_str(fb, 6, 469, "FRAME MS:", COL_BLUE);
    fb_fill_rect(fb, 64, 468, 194, 9, COL_BG_DARK);
    for (uint32_t i = 0; i < 64u; i++) {
        uint32_t ms = g_doom_tel.frame_hist[(g_doom_tel.hist_pos + i) & 63u];
        int bh = (int)(ms >> 2);
        if (bh < 1 && ms > 0u) bh = 1;
        if (bh > 8) bh = 8;
        uint16_t bc = (ms <= 30u) ? COL_GREEN : (ms <= 45u) ? COL_YELLOW : COL_RED_LIT;
        if (bh > 0) fb_fill_rect(fb, 65 + (int)i * 3, 476 - bh, 2, bh, bc);
    }
    snprintf(line, sizeof(line), "E%dM%d  GAMMA:%d  FRM:%u  [MODE: 4:3 CRT / HW EXPLORER]",
             gameepisode, gamemap, usegamma, (unsigned)g_doom_tel.frames_drawn);
    fb_draw_str(fb, 266, 469, line, COL_LIGHT);
}

struct peek_region {
    uint32_t    base_va;
    const char *title;
};

/* All 6 live hex-peek regions reside in hardware-proven SDRAM (zero MMIO read side-effects) */
static const struct peek_region g_peek_regions[6] = {
    { 0xA07FAA00u, "KEYPAD / SHUTTER / SWITCH MATRIX" },
    { 0xA07DD700u, "UI SCREEN / ARGB DIRTY / PSAVE STATE" },
    { 0xA0939684u, "ACTIVE CAMERA NVRAM MIRROR" },
    { 0xA1C00000u, "CFW PAYLOAD BASE & LV PLANES" },
    { 0xA1CC0000u, "D2AP DOOM.BIN HEADER & ENTRY" },
    { 0xA07FBC1Cu, "M6M ITCM EXCEPTION VECTOR IMAGE" },
};

static void draw_explorer_overlay(uint16_t *fb) {
    fb_draw_box(fb, 20, 50, 600, 378, COL_ACCENT, COL_PANEL_TR);
    fb_fill_rect(fb, 21, 51, 598, 18, COL_HDR_TR);
    fb_draw_str(fb, 30, 56,
                "DP2M LIVE HARDWARE & MEMORY EXPLORER  [QS+UP/DN: REGION | MODE: CLOSE]",
                COL_ACCENT);

    uint32_t cpsr = rd_cpsr();
    uint32_t mode_bits = cpsr & 0x1Fu;
    const char *cpu_mode = (mode_bits == 0x13u) ? "SVC"
                         : (mode_bits == 0x1Fu) ? "SYS"
                         : (mode_bits == 0x10u) ? "USR"
                         : (mode_bits == 0x12u) ? "IRQ" : "ARM";

    char line[104];
    int y = 76;
    snprintf(line, sizeof(line),
             "CPU   : ARM946E-S BE (%s IRQ:%s FIQ:%s)  CPSR:0x%08X  SP:0x%08X  ENTRY:0x%08X",
             cpu_mode, (cpsr & 0x80u) ? "MASK" : "EN", (cpsr & 0x40u) ? "MASK" : "EN",
             (unsigned)cpsr, (unsigned)rd_sp(), (unsigned)g_doom_tel.entry_sp);
    fb_draw_str(fb, 30, y, line, COL_WHITE); y += 12;

    snprintf(line, sizeof(line),
             "ARENA : LOAD 0xA1CC0000..0x%08X (%u KB)  BSS..0x%08X (%u KB)  LIMIT 0xA1E00000",
             (unsigned)(uintptr_t)__image_end,
             (unsigned)(((uintptr_t)__image_end - 0xA1CC0000u) >> 10),
             (unsigned)(uintptr_t)__bss_end,
             (unsigned)(((uintptr_t)__bss_end - (uintptr_t)__image_end) >> 10));
    fb_draw_str(fb, 30, y, line, COL_BLUE); y += 12;

    snprintf(line, sizeof(line),
             "CLOCK : 0x9000010C=0x%08X  0x90000110=0x%08X  TICK=%08X:%08X  OS_MS=%u",
             (unsigned)rd_reg32(0x9000010Cu),
             (unsigned)rd_reg32(0x90000110u),
             (unsigned)rd_reg32(0xA07F3788u),
             (unsigned)rd_reg32(0xA07F378Cu),
             (unsigned)(g_dp2m_api->get_ms ? g_dp2m_api->get_ms() : 0u));
    fb_draw_str(fb, 30, y, line, COL_LIGHT); y += 12;

    snprintf(line, sizeof(line),
             "CAM   : SCR=0x%02X  PSAVE=0x%08X  CM=%u  ISO=%u  EXPO=%u  ENG=%u  EXEC=%s",
             (unsigned)rd_active_screen_id(),
             (unsigned)rd_reg32(0xA07DD73Cu),
             (unsigned)rd_reg8(0xA09396C3u),
             (unsigned)rd_reg8(0xA09396DAu),
             (unsigned)rd_reg8(0xA09396ACu),
             (unsigned)rd_reg8(0xA0939689u),
             g_dp2m_api->is_autonomous ? "HW_UI_TASK" : "SIM_EVENT");
    fb_draw_str(fb, 30, y, line, COL_LIGHT); y += 12;

    snprintf(line, sizeof(line),
             "INPUT : KEY0=0x%08X  SW28=0x%08X  SNAP=0x%08X  DIAL=%u(%+d)  ACCEL=%+d/%+d",
             (unsigned)g_doom_tel.inp.raw_key0,
             (unsigned)g_doom_tel.inp.raw_sw28,
             (unsigned)rd_reg32(0xA07FAB20u),
             (unsigned)g_doom_tel.inp.raw_dial,
             (int)g_doom_tel.inp.dial_delta,
             (int)g_doom_tel.inp.accel_x,
             (int)g_doom_tel.inp.accel_y);
    fb_draw_str(fb, 30, y, line, COL_GREEN); y += 12;

    snprintf(line, sizeof(line),
             "MEM   : ZONE=%u KB  HEAP=%u KB (PEAK %u KB, %u BLKS)  WAD=%s (%s, OFF=0x%X)",
             (unsigned)(g_doom_tel.zone_bytes >> 10),
             (unsigned)(g_doom_tel.heap_bytes >> 10),
             (unsigned)(g_doom_tel.heap_peak_bytes >> 10),
             (unsigned)g_doom_tel.heap_blocks,
             g_doom_tel.wad_name,
             g_doom_tel.wad_preloaded ? "SDRAM" : "STREAM",
             (unsigned)g_doom_tel.wad_last_off);
    fb_draw_str(fb, 30, y, line, COL_PURPLE); y += 16;

    /* Divider + Live 128-byte Hex/ASCII Memory Inspector (8 rows x 16 bytes) */
    fb_fill_rect(fb, 30, y, 580, 1, COL_GRID); y += 6;
    uint32_t ridx = g_doom_tel.peek_idx % 6u;
    uint32_t base = g_peek_regions[ridx].base_va;
    snprintf(line, sizeof(line),
             "LIVE HEX PEEK [%u/6]: 0x%08X .. 0x%08X  --  %s",
             (unsigned)(ridx + 1u), (unsigned)base, (unsigned)(base + 0x7Fu),
             g_peek_regions[ridx].title);
    fb_draw_str(fb, 30, y, line, COL_CYAN); y += 14;

    for (uint32_t row = 0; row < 8u; row++, y += 12) {
        uint32_t va = base + row * 16u;
        uint32_t w0 = rd_reg32(va + 0u);
        uint32_t w1 = rd_reg32(va + 4u);
        uint32_t w2 = rd_reg32(va + 8u);
        uint32_t w3 = rd_reg32(va + 12u);
        uint8_t bytes[16];
        for (int b = 0; b < 4; b++) {
            bytes[0 + b]  = (uint8_t)(w0 >> (24 - b * 8));
            bytes[4 + b]  = (uint8_t)(w1 >> (24 - b * 8));
            bytes[8 + b]  = (uint8_t)(w2 >> (24 - b * 8));
            bytes[12 + b] = (uint8_t)(w3 >> (24 - b * 8));
        }
        char asc[17];
        for (int b = 0; b < 16; b++) {
            asc[b] = (bytes[b] >= 0x20u && bytes[b] <= 0x7Eu) ? (char)bytes[b] : '.';
        }
        asc[16] = '\0';
        snprintf(line, sizeof(line),
                 "%08X: %08X %08X %08X %08X  |%s|",
                 (unsigned)va, (unsigned)w0, (unsigned)w1, (unsigned)w2, (unsigned)w3, asc);
        fb_draw_str(fb, 30, y, line, (row & 1u) ? COL_LIGHT : COL_WHITE);
    }

    y += 6;
    fb_fill_rect(fb, 30, y, 580, 1, COL_GRID); y += 8;
    int health = players[consoleplayer].health;
    int armor  = players[consoleplayer].armorpoints;
    snprintf(line, sizeof(line),
             "DOOM  : STATE=%d  E%dM%d  HP=%d%%  ARM=%d%%  KILLS=%d/%d  SECRETS=%d/%d",
             (int)gamestate, gameepisode, gamemap, health, armor,
             players[consoleplayer].killcount, totalkills,
             players[consoleplayer].secretcount, totalsecret);
    fb_draw_str(fb, 30, y, line, COL_GREEN);
}

void dp2m_video_present_frame(void) {
    if (!I_VideoBuffer || !g_dp2m_api || !g_dp2m_api->argb_fb) return;
    uint16_t *fb = g_dp2m_api->argb_fb;

    uint32_t mode = g_doom_tel.hud_mode % DP2M_HUD_MODE_COUNT;
    if (mode == DP2M_HUD_FULLSCREEN) {
        blit_640x480_crt43(I_VideoBuffer, fb);
        char badge[16];
        snprintf(badge, sizeof(badge), "FPS:%2u", (unsigned)g_doom_tel.fps);
        fb_fill_rect(fb, 594, 4, 42, 11, COL_HDR_TR);
        fb_draw_str(fb, 597, 6, badge, COL_GREEN);
    } else {
        blit_640x400_centered(I_VideoBuffer, fb);
        draw_dashboard_bars(fb);
        if (mode == DP2M_HUD_EXPLORER)
            draw_explorer_overlay(fb);
    }

    g_doom_tel.frames_drawn++;
    g_dp2m_api->present_argb();
}

/* ---------- doomgeneric I_* Video Hooks ---------- */

void I_InitGraphics(void) {
    I_VideoBuffer = (byte *)Z_Malloc(SCREENWIDTH * SCREENHEIGHT, PU_STATIC, NULL);
    memset(I_VideoBuffer, 0, SCREENWIDTH * SCREENHEIGHT);
    screenvisible = true;
    I_InitInput();
}

void I_ShutdownGraphics(void) {
    if (I_VideoBuffer) {
        Z_Free(I_VideoBuffer);
        I_VideoBuffer = NULL;
    }
}

void I_StartFrame(void) {}

void I_StartTic(void) {
    I_GetEvent();
}

void I_UpdateNoBlit(void) {}

void I_FinishUpdate(void) {
    DG_DrawFrame();
}

void I_ReadScreen(byte *scr) {
    if (I_VideoBuffer && scr)
        memcpy(scr, I_VideoBuffer, SCREENWIDTH * SCREENHEIGHT);
}

void I_BeginRead(void) {}
void I_EndRead(void) {}
void I_EnableLoadingDisk(void) {}
void I_GraphicsCheckCommandLine(void) {}
void I_SetWindowTitle(char *title) { (void)title; }
void I_CheckIsScreensaver(void) {}
void I_SetGrabMouseCallback(grabmouse_callback_t func) { (void)func; }
void I_DisplayFPSDots(boolean dots_on) { (void)dots_on; }
void I_BindVideoVariables(void) {}
void I_InitWindowTitle(void) {}
void I_InitWindowIcon(void) {}
