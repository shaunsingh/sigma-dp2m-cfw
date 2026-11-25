// exposure modes C4/C5 (values 9/10): slots 3/4 live in payload RAM and persist after the stock record in NOR bank 0
#include "dp2m.h"
#include "catalog.h"

#define EXPO_MOVIE    4u
#define EXPO_C1       5u
#define EXPO_PLAYBACK 8u
#define EXPO_C4       9u
#define NO_SLOT       0xFFu

#define NV_CUR   0xA09396C0u           /* shooting settings, the unit a slot stores */
#define NV_SLOTS 0xA0939818u           /* C1..C3 */
#define ROM_CUR  (0xA07F9E68u + 0x3Cu) /* NVRAM defaults */
#define SLOT_SZ  0x8Cu
#define NOR_REC  0x400u                /* bank 0 offset; the stock record ends at 0x3E4 */
#define CS_MAGIC 0x43344335u           /* "C4C5" */

#define LABEL(cls, sid) ((cls) + (sid) - DP2M_SID0)

// RAM image of the NOR record
static struct {
    uint32_t magic;
    uint8_t slot[2][SLOT_SZ];
} cs;

static uint32_t cslot(uint32_t mode) {
    return mode - EXPO_C1 < 3u ? mode - EXPO_C1 : mode - EXPO_C4 < 2u ? mode - EXPO_C4 + 3u : NO_SLOT;
}

static void cs_reset(void) {
    cs.magic = CS_MAGIC;
    memcpy(cs.slot[0], (const void *)ROM_CUR, SLOT_SZ);
    memcpy(cs.slot[1], (const void *)ROM_CUR, SLOT_SZ);
}

// payload RAM is not cleared at boot and the boot load skips the copy if its buffer allocation fails
static void cs_ready(void) {
    if (cs.magic != CS_MAGIC) cs_reset();
}

uint8_t *slot_ptr(uint32_t i) {
    if (i < 3u) return (uint8_t *)NV_SLOTS + i * SLOT_SZ;
    cs_ready();
    return cs.slot[i - 3u];
}

static void slot_load(uint32_t i) {
    memcpy((void *)NV_CUR, slot_ptr(i), SLOT_SZ);
}

void slot_save(uint32_t i) {
    memcpy(slot_ptr(i), (const void *)NV_CUR, SLOT_SZ);
}
HOOK_CALL(0xA0059A7E, slot_save, 0xA0167584); // Custom Mode Setting "Yes"

// slot compare 0xA0167618 reads the slot relative to `NVRAM + 0x194`: `cmp r0,#3; bhs` -> bl slot_ptr; r0 -= 0x194
HOOK_BL(0xA016761C, slot_ptr, 0x2803D275);
HOOK_HALF(0xA0167622, 0x38FF, 0x4350);
HOOK_HALF(0xA0167628, 0x3895, 0x1880);

// backup on entering the custom range from 0..4, restore on leaving it for 0..4; returning from playback (8) only re-applies
void expo_switch(uint32_t old, uint32_t mode) {
    uint32_t s = cslot(mode);
    uint32_t from_custom = cslot(old) != NO_SLOT;
    if (s != NO_SLOT) {
        if (old != EXPO_PLAYBACK) {
            if (!from_custom) stock_cur_backup();
            slot_load(s);
        }
        stock_cur_apply();
        return;
    }
    if (mode == EXPO_PLAYBACK) return;
    if (mode == EXPO_MOVIE) stock_movie_apply();
    if (from_custom) {
        stock_cur_restore();
        stock_cur_apply();
    }
    if (mode != EXPO_MOVIE) stock_still_apply();
}

// setter 0xA016F9EC tail at 0xA016FA2E (`subs r3,r5,#4; blx switch8`): r4 old, r5 new, r6 = &shadow; stock pushed {r4-r6,lr}
__attribute__((naked)) void expo_switch_hook(void) {
    __asm__("movs r0, r4\n"
            "movs r1, r5\n"
            "bl   expo_switch\n"
            "strb r5, [r6]\n"
            "pop  {r4, r5, r6, pc}");
}
HOOK_BL(0xA016FA2E, expo_switch_hook, 0x1F2BF003);

// program resolver 0xA01675DA after `push {lr}`: r3 = mode; replaces `movs r0,#3; movs r2,r3` so 9/10 take the C1..C3 case
__attribute__((naked)) void program_hook(void) {
    __asm__("movs r0, #3\n"
            "movs r2, r3\n"
            "subs r3, #9\n"
            "cmp  r3, #1\n"
            "bls  1f\n"
            "movs r3, r2\n"
            "bx   lr\n"
            "1:\n"
            "movs r3, #5\n"
            "bx   lr");
}
HOOK_BL(0xA01675E2, program_hook, 0x2003001A);

void expo_boot_reload(void) {
    uint32_t s = cslot(stock_get_expo_mode());
    if (s != NO_SLOT) slot_load(s);
}
HOOK_CALL(0xA01689C2, expo_boot_reload, 0xA016F400);

// stock "is custom" checks only know 5..7
uint32_t expo_mode_c1(void) {
    uint32_t m = stock_get_expo_mode();
    return cslot(m) != NO_SLOT ? EXPO_C1 : m;
}
HOOK_CALL(0xA016A7FC, expo_mode_c1, 0xA0167892); // LV widget 0x2E visibility
HOOK_CALL(0xA0169884, expo_mode_c1, 0xA0167892); // still-mode apply switch
HOOK_CALL(0xA016BAE8, expo_mode_c1, 0xA0167892); // widget 0x2E draw: every custom mode takes the C1 path

static const uint16_t widget_icons[5] = {
    0xB7, 0xB8, 0xB9, LABEL(ICON_LABEL_WIDGET, SID_C4), LABEL(ICON_LABEL_WIDGET, SID_C5),
};

const void *widget_icon(void) {
    return stock_icon(widget_icons[cslot(stock_get_expo_mode())]);
}
HOOK_CALL(0xA016BC26, widget_icon, 0xA0042AAC);

uint32_t widget_slot_same(void) {
    return stock_slot_same(cslot(stock_get_expo_mode()));
}
HOOK_CALL(0xA016BBBA, widget_slot_same, 0xA0167618);

// MODE screen (23): ring Movie, M, S, A, P, C1..C5; the screen keeps its u16 cursor in the word before the cells
struct mode_cell {
    uint32_t mode;
    uint16_t icon, pad;
};
static struct {
    uint16_t cursor, pad;
    struct mode_cell cell[11];
} mode_strip __attribute__((section(".menu.mode_strip"))) = { .cell = {
    { 4, 0xAF, 0 }, { 0, 0xB0, 0 }, { 1, 0xB1, 0 }, { 2, 0xB2, 0 }, { 3, 0xB3, 0 },
    { 5, 0xB4, 0 }, { 6, 0xB5, 0 }, { 7, 0xB6, 0 },
    { 9, LABEL(ICON_LABEL_MODE, SID_C4), 0 }, { 10, LABEL(ICON_LABEL_MODE, SID_C5), 0 },
} };
HOOK_WORD(0xA005247C, mode_strip.cell, 0xA07F215C);
HOOK_HALF(0xA00522A2, 0x2C0A, 0x2C08); // draw loop
HOOK_HALF(0xA00522EC, 0x2109, 0x2107); // cursor max, both directions
HOOK_HALF(0xA0052342, 0x2109, 0x2107);
HOOK_HALF(0xA00523AC, 0x280A, 0x2808); // cursor search on open
// x0 = (X - 8w) / 2 with cell w = 60 (rect 0xA04F30A0): X = 640 -> 520 centers 10 cells at 20..620
HOOK_HALF(0xA005220E, 0x2082, 0x2005);
HOOK_HALF(0xA0052210, 0x0080, 0x01C0);

// Custom Mode Setting screen (31): 5 slots in 120 px cells from x = 20 (stock 3 x 188 px from 36)
static const uint16_t custom_icons[5] __attribute__((aligned(4))) = {
    0xB4, 0xB5, 0xB6, LABEL(ICON_LABEL_MODE, SID_C4), LABEL(ICON_LABEL_MODE, SID_C5),
};
HOOK_WORD(0xA0059AE0, custom_icons, 0xA04F3188);
HOOK_HALF(0xA0059898, 0x4991, 0xA909); // icons[i] through the pool word, not the 4-entry stack copy
HOOK_HALF(0xA0059914, 0x4872, 0xA809);
HOOK_HALF(0xA00598F4, 0x2C05, 0x2C03); // draw loop
HOOK_HALF(0xA00599E4, 0x2104, 0x2102); // cursor max, both directions
HOOK_HALF(0xA0059A02, 0x2104, 0x2102);
HOOK_HALF(0xA00597D6, 0x2178, 0x21BC); // cursor box
HOOK_HALF(0xA00597DC, 0x3014, 0x3024);
HOOK_HALF(0xA00597F0, 0x2178, 0x21BC); // left arrow, 6 px outside the box
HOOK_HALF(0xA00597F6, 0x3001, 0x3011);
HOOK_HALF(0xA0059838, 0x2178, 0x21BC); // right arrow
HOOK_HALF(0xA005983E, 0x3092, 0x30E6);
HOOK_HALF(0xA0059880, 0x2178, 0x21BC); // icon rect, 58 px centered in the cell
HOOK_HALF(0xA0059886, 0x3033, 0x3065);

// boot load 0xA0040110 reads the whole 2 KB bank into buf and copies buf + 0x20 into NVRAM
void nv_load_copy(void *dst, const uint8_t *src, uint32_t n) {
    memcpy(dst, src, n);
    memcpy(&cs, src - 0x20 + NOR_REC, sizeof cs);
    if (cs.magic != CS_MAGIC) cs_reset();
}
HOOK_CALL(0xA0040168, nv_load_copy, 0xA000EE68);

// flush 0xA004017A writes buf to bank 0 from `off` = 8: extend it over a zeroed gap and the C4/C5 record
void nv_flush_write(uint32_t off, uint32_t n, uint8_t *buf) {
    cs_ready();
    memset(buf + n, 0, NOR_REC - off - n);
    memcpy(buf + NOR_REC - off, &cs, sizeof cs);
    stock_nor_write(off, NOR_REC - off + sizeof cs, buf);
}
HOOK_CALL(0xA00401E6, nv_flush_write, 0xA00206FA);

// camera reset 0xA0167574 restores the persisted block (incl. C1..C3) from ROM
void nv_reset_copy(void *dst, const void *src, uint32_t n) {
    memcpy(dst, src, n);
    cs_reset();
}
HOOK_CALL(0xA016757E, nv_reset_copy, 0xA000EE68);
