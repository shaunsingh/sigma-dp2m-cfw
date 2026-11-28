#ifndef DP2M_H
#define DP2M_H

#include <stddef.h>
#include <stdint.h>

/* rt.c */
void *memcpy(void *d, const void *s, size_t n);
void *memset(void *d, int c, size_t n);

/* stock icon lookup result: 8bpp tile, rows of w */
struct icon { const uint8_t *data; uint16_t w, h; };

/* stock M6M ABI (S3, Thumb unless noted); symbols are absolute, defined in stock.S */
uint32_t stock_get_cm(void);
uint32_t stock_get_grid(void);
uint32_t stock_get_expo_mode(void);
uint32_t stock_ui_screen_id(void);
uint32_t stock_lv_active(void);
void stock_lv_refresh(void);
void stock_lv_mono(uint32_t lv_mode, uint32_t cm);
void stock_lv_cmcm(uint32_t lv_mode, uint32_t cm);
void stock_get_lv_cc7(void *buf);
void stock_get_lv_ccn(void *buf);
void stock_grid_draw(uint32_t arg);
void stock_fill_rect(const uint16_t *rect, const uint16_t *rgba);
void stock_flush_rect(int32_t x0, int32_t y0, int32_t x1, int32_t y1);
uint32_t stock_accel_x(void);
uint32_t stock_accel_y(void);
void stock_b2y_set_mat(void *mat);                                   /* ARM */
void stock_b2y_set_gamma(uint32_t ch, const void *lut, uint32_t sz); /* ARM */
const struct icon *stock_icon(uint32_t id);                          /* 0 = unknown id */
void stock_nor_write(uint32_t off, uint32_t n, const void *buf);     /* NOR bank 0 read-modify-write */
void stock_cur_backup(void);                                         /* shooting settings -> pre-custom backup */
void stock_cur_restore(void);                                        /* pre-custom backup -> shooting settings */
void stock_cur_apply(void);                                          /* re-apply the shooting settings */
uint32_t stock_slot_same(uint32_t slot);                             /* 1 = custom slot matches the shooting settings */
void stock_still_apply(void);
void stock_movie_apply(void);
void stock_lv_status_cm(uint32_t cm);
void stock_pb_info_cm(uint32_t cm);
/* weight 0..9 (8+ bold), outline px (draws over thin strokes), narrow = 0.75 width; rect y >= 480 = line buffer only */
uint32_t stock_text(uint32_t lang, const char *s, int32_t n, uint32_t px, uint32_t weight, uint32_t outline,
                    uint32_t narrow, const uint16_t *rect, uint32_t style, uint32_t z);
void stock_font_scale(uint32_t num, uint32_t den); /* text width x num / den */
void stock_disp_init(void);                        /* boot: LV display setup per DISP mode */
void stock_screen_post(uint32_t type, uint32_t arg);
void stock_screen_goto(uint32_t screen_id);
void stock_osd_clear(void);
void stock_osd_commit(void);
void stock_argb_clear(void);
void stock_argb_commit(void);
uint32_t stock_busy_check(void);
void *stock_calloc(size_t n, size_t sz);
void stock_free(void *p);
void *stock_malloc(size_t sz);
void *stock_realloc(void *p, size_t sz);
void stock_psave_inhibit(uint32_t mask);
void stock_psave_release(uint32_t mask);
int32_t stock_rel_mpf(int32_t mpfid, void *blf);                    /* ARM */
int32_t stock_prcv_mbx(int32_t mbxid, void **ppk_msg);              /* ARM */
int32_t stock_snd_mbx(int32_t mbxid, void *pk_msg);                 /* ARM */
int32_t stock_get_tid(uint16_t *p_tskid);                           /* ARM */
int32_t stock_dly_tsk(uint32_t ms);                                 /* ARM */
int32_t stock_get_tim(uint64_t *p_systim);                          /* ARM; p_systim MUST be 8B-aligned */
int32_t stock_mdf_open(const char *path, uint32_t flags, uint32_t mode);
int32_t stock_mdf_close(int32_t fd);
int32_t stock_mdf_read(int32_t fd, void *buf, uint32_t len, uint32_t *got);
int32_t stock_mdf_fstat(int32_t fd, void *stat_buf);
int32_t stock_mdf_lseek(int32_t fd, int32_t off, uint32_t whence, uint32_t *pos);

/* ITCM SWI 9 (0x20C, image 0xA07FBE28): clean the 16 KB D-cache by index and drain the write buffer. */
static inline void hw_dcache_clean(void) {
    __asm__ volatile("svc #9" ::: "lr", "memory");
}

/* ITCM SWI 10 (0x248, image 0xA07FBE64): clean the 16 KB D-cache by index, drain the write buffer, invalidate the
 * D-cache; the I-cache is untouched. The handler restores r0-r2 and the flags; svc overwrites lr in SVC mode. */
static inline void hw_dcache_sync(void) {
    __asm__ volatile("svc #10" ::: "lr", "memory");
}

/* ITCM SWI 15 (0x310, image 0xA07FBF2C): invalidate entire 16 KB I-cache (restores r0-r1 and flags). */
static inline void hw_icache_inv_all(void) {
    __asm__ volatile("svc #15" ::: "lr", "memory");
}

/* ITCM SWI 12 (0x2B0, image 0xA07FBECC): clean 32-byte D-cache lines over r0..r0+r1, drain the
 * write buffer, and preserve r0/r1 and flags. Unlike SWI 10, unrelated cache lines are untouched. */
static inline void hw_dcache_clean_range(const void *p, uint32_t n) {
    register uint32_t a __asm__("r0") = (uintptr_t)p & ~31u;
    register uint32_t z __asm__("r1") = n + ((uintptr_t)p & 31u);
    if (n) __asm__ volatile("svc #12" : "+r"(a), "+r"(z) :: "lr", "memory");
}

#define REG8(a)  (*(volatile uint8_t *)(a))
#define REG16(a) (*(volatile uint16_t *)(a))
#define REG32(a) (*(volatile uint32_t *)(a))

/* NVRAM block 0xA0939684 (persisted 0x3C4 B); +0x04..+0x13 is reserved (zero defaults); +0x0F..+0x13 unused */
#define NV_COLOR_PAPER 0xA0939688u /* 0 = film's Auto paper, 1..8 */
#define NV_ENGINE      0xA0939689u /* 0 = HW fast, 1 = 3D spectral */
#define NV_FILL        0xA093968Au /* X3 fill 0..3 */
#define NV_MINSHUTTER  0xA093968Bu /* 0..6 */
#define NV_PEAKCOL     0xA093968Cu /* 0 red, 1 yellow, 2 white, 3 black */
#define NV_GRAIN       0xA093968Du /* 0/1 */
#define NV_HALATION    0xA093968Eu /* 0/1 */
#define NV_BW_PAPER    0xA093968Fu /* 0 = Auto, 1..8 */
#define NV_LUT_SLOT    0xA0939690u /* 0 = Auto, 1..8 */
#define NV_PEAKSENS    0xA0939691u /* 0 = Med (PEAKING_THRESH, def 44), 1 = High (24), 2 = Low (72), 3 = Auto (MF High / 1x Med) */
#define NV_ZEBRALVL    0xA0939692u /* 0 = 95% (ZEBRA_THRESH, def 242), 1 = 90% Safe (230), 2 = 100% Clip (252), 3 = Dual (Hi + Lo<=12) */

#define MF_SPOT_ACTIVE 0xA0939A53u /* stock MF magnification active flag (0xA0939A44 + 0xF, getter 0xA01681CA) */

#define ARGB_FB        ((uint16_t *)0xA21D8C00u)

#define SCREEN_LV      0x15u
#define SCREEN_DOOM    0x21u
#define FILM_MODE0     9u

struct menu_row {
    uint32_t id;     /* low 16: submenu id for page rows */
    uint32_t flags;  /* 0x01000000 | 110x70 QS label icon */
    uint32_t icons;  /* (76x26 OSD icon << 16) | 54x42 QS strip icon */
    uint32_t sid;    /* string id << 16 */
    uint32_t value;
    uint32_t (*get)(void);
    void (*set)(uint32_t);
    uint32_t (*extra)(void);
};

/* per on-camera film (color mode - FILM_MODE0), generated by the builder */
struct film_info {
    uint8_t packed;     /* M5 packed mode at paper 0 */
    uint8_t paper_cls;  /* 0 none, 1 color, 2 bw, 3 lut */
    uint8_t auto_paper; /* 0-based */
    uint8_t mono;
};

/* live view look per packed mode >= FILM_MODE0 */
struct lv_look {
    int16_t cmcm[9];   /* q10 left-multiply, or q7 direct weights when LV_MONO_Q7 */
    uint8_t knots[18]; /* 18-knot gamma spline; all-zero keeps stock gamma */
    uint8_t flags;
    uint8_t _pad;
};
#define LV_CMCM    1u
#define LV_MONO_Q7 2u
#define LV_KNOTS   4u

/* generated tables (catalog.c) */
extern const struct film_info dp2m_films[];
extern const uint32_t dp2m_film_count;
extern const struct lv_look dp2m_lv_looks[];
extern const char *const dp2m_strings[];
extern const uint32_t dp2m_string_count;
extern const uint32_t dp2m_tv_min_q16[7];
struct menu_map { uint16_t id; uint16_t kind; const struct menu_row *rows; };
extern const struct menu_map dp2m_menus[];

#define DP2M_SID0 672u

uint32_t packed_cm(uint32_t cm);
const struct menu_row *menu_lookup(uint32_t id);

/* build-time patches: verified against `expect` then applied by the builder */
struct dp2m_hook {
    uint32_t kind;
    uint32_t site;
    const void *target;
    uint32_t expect;
};
#define HK_CALL 1 /* retarget the BL/BLX at site; expect = original callee */
#define HK_BL   2 /* write a BL at site over 4 stock bytes `expect` */
#define HK_WORD 3 /* 32-bit value over `expect` */
#define HK_HALF 4 /* 16-bit value over `expect` */

#define DP2M_CAT_(a, b) a##b
#define DP2M_CAT(a, b) DP2M_CAT_(a, b)
#define DP2M_HOOK(k, s, t, e) \
    __attribute__((used, section(".dp2m.hooks"))) static const struct dp2m_hook DP2M_CAT(dp2m_hook_, __COUNTER__) = { (k), (s), (const void *)(t), (e) }
#define HOOK_CALL(site, fn, old) DP2M_HOOK(HK_CALL, site, fn, old)
#define HOOK_BL(site, fn, stock) DP2M_HOOK(HK_BL, site, fn, stock)
#define HOOK_WORD(site, val, stock) DP2M_HOOK(HK_WORD, site, val, stock)
#define HOOK_HALF(site, val, stock) DP2M_HOOK(HK_HALF, site, val, stock)

/* live-view YCC plane pointers & frame count written by lv_ycap_hook (hooks.S) */
struct lv_planes { uint32_t y, cb, cr, fcnt; };
extern volatile struct lv_planes lv_planes;

static inline const struct film_info *film_of(uint32_t cm) {
    return (cm - FILM_MODE0 < dp2m_film_count) ? &dp2m_films[cm - FILM_MODE0] : 0;
}

const struct icon *label_icon(uint32_t id);
uint32_t osd_map(uint32_t cm);
void lv_boot_reset(void);
void doom_boot_reset(void);

#endif
