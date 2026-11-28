/* SCREEN_DOOM (0x21): loads the SD applet DOOM.BIN ("D2AP" v1) and runs it on the UI task */
#include "dp2m.h"
#include "dp2m_app.h"

extern uint8_t __bss_end[];

#define PSAVE_DOOM_MASK 0x40000000u /* Exclusive power-save inhibit bit */
#define DOOM_MBX_UI     0x11
#define DOOM_MPF_UI     0x10
#define APP_ACTIVE_TAG  0x44324131u /* "D2A1" */

static struct {
    volatile uint32_t active_magic;
    dp2m_app_entry_fn entry;
    uint8_t dial_prev;
    uint8_t rls_inherit; /* DP2M_RLS_SRC_* readings still held from before DOOM opened */
} app_rt;

static struct dp2m_app_api app_api_tbl;

static void app_present_argb(void) {
    hw_dcache_sync();
    stock_flush_rect(0, 0, 640, 480);
    stock_argb_commit();
}

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

static void app_sleep_ms(uint32_t ms) {
    if (ms) stock_dly_tsk(ms);
}

static int32_t app_mdf_read(int32_t fd, void *buf, uint32_t len, uint32_t *got) {
    hw_dcache_sync();
    int32_t rc = stock_mdf_read(fd, buf, len, got);
    hw_dcache_sync();
    return rc;
}

static void app_poll_input(struct dp2m_input_snapshot *out) {
    /* Stock polarity (0xA016D88C..0xA016DB64, 0xA016E47A):
     * - 0xA07FAA1C (KEY0): 0x00000000 when idle; 2-bit slot (k0 >> (30 - 2*k)) & 3
     *   is 1 on press and 3 on auto-repeat for k = 0..11 (OK,UP,DN,LF,RT,MENU,DISP,AEL,QS,MODE,DEL,PLAY).
     * - 0xA07FAA28 (SW28, built by 0xA016DD94) carries no shutter contact: bits 31/26 follow GPIO pins
     *   0x44/0x43 and drive the stock handlers 0xA0168F04/0xA016901C, bit 30 follows pin 0x42 and
     *   bits 19..25 copy the level byte at 0xA07DD74C. It is reported raw for the HUD only.
     * - The shutter comes from the release switches (dp2m_release_sources). A reading that is already
     *   active when DOOM opens stays ignored until it clears, so a line that never idles cannot hold
     *   the fire button down. */
    uint32_t k0 = REG32(0xA07FAA1Cu);
    uint32_t sw = REG32(0xA07FAA28u);
    uint32_t dial = REG8(0xA07AF38Cu);
    uint32_t mask = 0u;
    for (uint32_t k = 0u; k < 12u; k++) {
        if (((k0 >> (30u - 2u * k)) & 3u) != 0u)
            mask |= (1u << k);
    }
    uint32_t rls = dp2m_release_sources();
    app_rt.rls_inherit &= (uint8_t)rls;
    if ((rls & ~(uint32_t)app_rt.rls_inherit) != 0u)
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

static void app_input_reset(void) {
    app_rt.dial_prev = REG8(0xA07AF38Cu);
    app_rt.rls_inherit = (uint8_t)dp2m_release_sources();
}

void doom_boot_reset(void) {
    memset(&app_rt, 0, sizeof(app_rt));
}

static const struct dp2m_app_api *app_build_api(uint32_t autonomous) {
    app_api_tbl.abi_ver = DP2M_APP_ABI_VER;
    app_api_tbl.is_autonomous = autonomous;
    app_api_tbl.argb_fb = ARGB_FB;
    app_api_tbl.present_argb = app_present_argb;
    app_api_tbl.reserved0 = 0;
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

/* Returns 0 once DOOM.BIN is loaded and its INIT succeeded, -1 otherwise */
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

    app_input_reset();
    dp2m_app_entry_fn entry = (dp2m_app_entry_fn)(uintptr_t)hdr.entry_va;
    const struct dp2m_app_api *api = app_build_api(autonomous);
    if (entry(api, DP2M_APP_INIT, 0u) != 0) {
        return -1;
    }
    app_rt.entry = entry;
    app_rt.active_magic = APP_ACTIVE_TAG;
    return 0;
}

static int doom_app_active(void) {
    return app_rt.active_magic == APP_ACTIVE_TAG && app_rt.entry;
}

static void doom_clear_planes(void) {
    stock_argb_clear();
    stock_argb_commit();
    stock_osd_clear();
    stock_osd_commit();
}

static void doom_shutdown_app_if_active(void) {
    if (doom_app_active()) {
        app_rt.entry(&app_api_tbl, DP2M_APP_SHUTDOWN, 0u);
    }
    app_rt.active_magic = 0u;
    app_rt.entry = 0;
}

/* ---------- Notice shown when DOOM.BIN or its WAD cannot be loaded ---------- */

/* 5x7 glyphs for the notice text only; bit 4 is the left column */
static const char notice_chars[] = "ABCDEFHIMNOPRTUWY.";
static const uint8_t notice_font[sizeof(notice_chars) - 1u][7] = {
    { 0x0E, 0x11, 0x11, 0x1F, 0x11, 0x11, 0x11 }, /* A */
    { 0x1E, 0x11, 0x11, 0x1E, 0x11, 0x11, 0x1E }, /* B */
    { 0x0E, 0x11, 0x10, 0x10, 0x10, 0x11, 0x0E }, /* C */
    { 0x1E, 0x11, 0x11, 0x11, 0x11, 0x11, 0x1E }, /* D */
    { 0x1F, 0x10, 0x10, 0x1E, 0x10, 0x10, 0x1F }, /* E */
    { 0x1F, 0x10, 0x10, 0x1E, 0x10, 0x10, 0x10 }, /* F */
    { 0x11, 0x11, 0x11, 0x1F, 0x11, 0x11, 0x11 }, /* H */
    { 0x0E, 0x04, 0x04, 0x04, 0x04, 0x04, 0x0E }, /* I */
    { 0x11, 0x1B, 0x15, 0x15, 0x11, 0x11, 0x11 }, /* M */
    { 0x11, 0x19, 0x15, 0x13, 0x11, 0x11, 0x11 }, /* N */
    { 0x0E, 0x11, 0x11, 0x11, 0x11, 0x11, 0x0E }, /* O */
    { 0x1E, 0x11, 0x11, 0x1E, 0x10, 0x10, 0x10 }, /* P */
    { 0x1E, 0x11, 0x11, 0x1E, 0x14, 0x12, 0x11 }, /* R */
    { 0x1F, 0x04, 0x04, 0x04, 0x04, 0x04, 0x04 }, /* T */
    { 0x11, 0x11, 0x11, 0x11, 0x11, 0x11, 0x0E }, /* U */
    { 0x11, 0x11, 0x11, 0x15, 0x15, 0x1B, 0x11 }, /* W */
    { 0x11, 0x11, 0x0A, 0x04, 0x04, 0x04, 0x04 }, /* Y */
    { 0x00, 0x00, 0x00, 0x00, 0x00, 0x0C, 0x0C }, /* . */
};

static void notice_text(int32_t x, int32_t y, const char *s, int32_t scale, uint16_t col) {
    for (; *s; s++, x += 6 * scale) {
        uint32_t g = 0u;
        while (notice_chars[g] && notice_chars[g] != *s)
            g++;
        if (!notice_chars[g])
            continue;
        for (int32_t r = 0; r < 7; r++) {
            for (int32_t c = 0; c < 5; c++) {
                if (!(notice_font[g][r] & (0x10u >> c)))
                    continue;
                uint16_t *p = ARGB_FB + (y + r * scale) * 640 + x + c * scale;
                for (int32_t dy = 0; dy < scale; dy++)
                    for (int32_t dx = 0; dx < scale; dx++)
                        p[dy * 640 + dx] = col;
            }
        }
    }
}

static void doom_draw_notice(void) {
    for (uint32_t i = 0u; i < 640u * 480u; i++)
        ARGB_FB[i] = 0x111Fu;
    notice_text(131, 205, "DOOM.BIN OR WAD ERROR", 3, 0xF7BFu);
    notice_text(194, 250, "COPY BOTH TO THE CARD", 2, 0xDDEFu);
    app_present_argb();
}

/* Poll MBX 0x11 each autonomous frame so MPF 0x10 never starves and system power-off
 * packets (type == 6, 0xA004330C) break out immediately without being discarded. */
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

/* Holds the notice for about 5 s, or until a button that was up when it appeared is pressed */
static void doom_notice_wait(void **sys_msg) {
    doom_draw_notice();
    app_input_reset();
    uint32_t prev_held = 0xFFFFFFFFu;
    for (uint32_t i = 0u; i < 150u; i++) {
        if (doom_poll_ui_mbx(sys_msg, 0))
            break;
        struct dp2m_input_snapshot inp;
        app_poll_input(&inp);
        if (inp.held_mask & ~prev_held)
            break;
        prev_held = inp.held_mask;
        app_sleep_ms(33u);
    }
}

static void doom_hw_exit_to_lv(void *sys_msg) {
    stock_psave_release(PSAVE_DOOM_MASK);
    /* Sync the SW28 & key edge snapshots so changes made during DOOM never replay as LV edges */
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

static void doom_leave_to_lv(void) {
    doom_shutdown_app_if_active();
    doom_clear_planes();
    stock_screen_goto(SCREEN_LV);
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
                /* The camera's UI task stays in this call until DOOM exits */
                void *sys_msg = 0;
                stock_psave_inhibit(PSAVE_DOOM_MASK);
                if (doom_try_load_app(1u) == 0) {
                    while (!doom_poll_ui_mbx(&sys_msg, 1) &&
                           app_rt.entry(&app_api_tbl, DP2M_APP_STEP, 0u) == 0) {}
                    doom_shutdown_app_if_active();
                } else {
                    doom_notice_wait(&sys_msg);
                }
                doom_hw_exit_to_lv(sys_msg);
                return 0u;
            }
            if (doom_try_load_app(0u) != 0)
                doom_draw_notice();
        } else if (arg == 3u && doom_app_active()) {
            if (app_rt.entry(&app_api_tbl, DP2M_APP_STEP, 0u) != 0)
                doom_leave_to_lv();
        }
        return 0u;
    }

    if (msg == 3u) {
        doom_shutdown_app_if_active();
        doom_clear_planes();
        return 0u;
    }

    if (msg == 5u) {
        /* Simulator key event: one applet step per key, or any key dismisses the notice */
        if (!doom_app_active() || app_rt.entry(&app_api_tbl, DP2M_APP_STEP, arg) != 0)
            doom_leave_to_lv();
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
