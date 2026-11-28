/* SPDX-License-Identifier: GPL-2.0-or-later */
/* Internal DP2M DOOM Applet Runtime, Telemetry & Hardware Explorer State */
#ifndef DP2M_DOOM_H
#define DP2M_DOOM_H

#include <stddef.h>
#include <stdint.h>
#include "dp2m_app.h"

/* ARMv5 setjmp/longjmp buffer: r4..r11, sp, lr (10 x 32-bit words) */
typedef uint32_t dp2m_jmp_buf[10];

int dp2m_setjmp(dp2m_jmp_buf env) __attribute__((returns_twice));
void dp2m_longjmp(dp2m_jmp_buf env, int val) __attribute__((noreturn));

enum {
    DP2M_HUD_DASHBOARD  = 0, /* 640x400 Bayer-dithered DOOM + 40px top/bottom DP2M telemetry */
    DP2M_HUD_FULLSCREEN = 1, /* 640x480 authentic 4:3 CRT aspect stretch */
    DP2M_HUD_EXPLORER   = 2, /* Semi-transparent live DP2M hardware & memory inspector */
    DP2M_HUD_MODE_COUNT = 3,
};

#define DP2M_PEEK_REGIONS 7u /* live hex-peek regions in the explorer */

struct dp2m_doom_telemetry {
    char     wad_name[16];
    uint32_t wad_size_bytes;
    uint32_t wad_preloaded;
    uint32_t wad_read_calls;
    uint32_t wad_read_bytes;
    uint32_t wad_last_off;
    uint32_t zone_bytes;
    uint32_t heap_bytes;
    uint32_t heap_peak_bytes;
    uint32_t heap_blocks;
    uint32_t hud_mode;
    uint32_t tilt_strafe;
    uint32_t peek_idx;
    uint32_t fps;
    uint32_t frame_ms;
    uint32_t frames_drawn;
    uint32_t entry_sp;
    uint32_t last_pc;
    uint8_t  frame_hist[64];
    uint32_t hist_pos;
    char     last_err[96];
    struct dp2m_input_snapshot inp;
    uint32_t raw_rls_state; /* DP2M_RLS_STATE[0] << 8 | DP2M_RLS_STATE[1] */
    uint32_t raw_rls_port;  /* REG32(DP2M_RLS_PORT) */
};

extern const struct dp2m_app_api *g_dp2m_api;
extern struct dp2m_doom_telemetry g_doom_tel;
extern dp2m_jmp_buf g_doom_exit_jmp;
extern volatile int g_doom_jmp_armed;

/* Tracked heap & file cleanup */
void dp2m_libc_reset(void);
void dp2m_free_all(void);
void dp2m_close_all_files(void);
int32_t dp2m_sd_open_read(const char *path);
void dp2m_sd_close(int32_t fd);

/* Boot splash & video presentation */
void dp2m_draw_boot_progress(const char *stage, uint32_t done, uint32_t total);
void dp2m_video_present_frame(void);

#endif
