/* SPDX-License-Identifier: GPL-2.0-or-later */
/* D2AP v1 Entrypoint, Hardware Chording & 35 Hz Loop for Standalone DOOM.BIN */
#include <stdint.h>
#include <stdio.h>
#include <stdlib.h>
#include <string.h>

#include "config.h"
#include "d_event.h"
#include "d_main.h"
#include "doomkeys.h"
#include "doomstat.h"
#include "doomgeneric.h"
#include "g_game.h"
#include "i_system.h"
#include "i_timer.h"
#include "i_video.h"
#include "m_argv.h"
#include "dp2m_doom.h"

extern uint8_t __image_end[];
extern uint8_t __bss_end[];
extern void D_DoomMain(void);

#define DP2M_APP_STACK_SIZE 8192u
#define DP2M_STACK_GUARD_MAGIC 0x44325347u /* "D2SG" */

/* DOOM must not depend on the undocumented free space in the stock UI task stack. */
static volatile uint32_t g_app_stack[DP2M_APP_STACK_SIZE / 4u]
    __attribute__((aligned(8), used));
static int32_t dp2m_doom_entry_body(const struct dp2m_app_api *api, uint32_t cmd, uint32_t arg)
    __attribute__((noinline, used));
int32_t dp2m_doom_entry(const struct dp2m_app_api *api, uint32_t cmd, uint32_t arg);

/* 32-byte D2AP v1 header at 0xA1CC0000 */
__attribute__((section(".dp2m.hdr"), used))
const struct dp2m_app_hdr g_dp2m_hdr = {
    DP2M_APP_MAGIC,
    DP2M_APP_ABI_VER,
    DP2M_APP_LOAD_VA,
    (uint32_t)(uintptr_t)&dp2m_doom_entry,
    (uint32_t)(uintptr_t)&__image_end - DP2M_APP_LOAD_VA,
    (uint32_t)(uintptr_t)&__bss_end,
    0u,
    0u,
};

/* Enter on a private 8-byte-aligned stack. The saved stock SP lives below the
 * private top for the whole call, including setjmp/longjmp error unwinds. */
__attribute__((naked))
int32_t dp2m_doom_entry(const struct dp2m_app_api *api, uint32_t cmd, uint32_t arg) {
    __asm__ volatile(
        "push  {r4-r11, lr}\n\t"
        "mov   r12, sp\n\t"
        "ldr   r3, =g_app_stack + 8192\n\t"
        "mov   sp, r3\n\t"
        "sub   sp, sp, #8\n\t"
        "str   r12, [sp]\n\t"
        "bl    dp2m_doom_entry_body\n\t"
        "ldr   r3, =g_app_stack\n\t"
        "ldr   r3, [r3]\n\t"
        "ldr   r2, =0x44325347\n\t"
        "cmp   r3, r2\n\t"
        "movne r0, #-1\n\t"
        "ldr   r12, [sp]\n\t"
        "mov   sp, r12\n\t"
        "pop   {r4-r11, pc}\n\t"
    );
}

const struct dp2m_app_api *g_dp2m_api = NULL;
struct dp2m_doom_telemetry g_doom_tel;
dp2m_jmp_buf g_doom_exit_jmp;
volatile int g_doom_jmp_armed = 0;

/* Dummy symbol in case any external reference inspects DG_ScreenBuffer */
pixel_t *DG_ScreenBuffer = NULL;

/* Key event ring buffer fed into DG_GetKey() */
#define KEYQ_LEN 64
static struct {
    uint8_t pressed;
    uint8_t key;
} g_keyq[KEYQ_LEN];
static uint32_t g_keyq_rd = 0u;
static uint32_t g_keyq_wr = 0u;

/* Virtual DOOM key bitmask for clean press/release tracking across chord changes */
enum {
    VK_UP       = 0,
    VK_DOWN     = 1,
    VK_LEFT     = 2,
    VK_RIGHT    = 3,
    VK_STRAFE_L = 4,
    VK_STRAFE_R = 5,
    VK_FIRE     = 6,
    VK_USE      = 7,
    VK_ENTER    = 8,
    VK_ESCAPE   = 9,
    VK_TAB      = 10,
    VK_RUN      = 11,
    VK_COUNT    = 12,
};

static const uint8_t g_vk_doom[VK_COUNT] = {
    KEY_UPARROW,
    KEY_DOWNARROW,
    KEY_LEFTARROW,
    KEY_RIGHTARROW,
    KEY_STRAFE_L,
    KEY_STRAFE_R,
    KEY_FIRE,
    KEY_USE,
    KEY_ENTER,
    KEY_ESCAPE,
    KEY_TAB,
    KEY_RSHIFT,
};

static uint32_t g_prev_vk_mask = 0u;
static uint32_t g_prev_hw_mask = 0xFFFFFFFFu;
static uint32_t g_boot_held = 0u;
static uint8_t  g_boot_primed = 0u;
static uint32_t g_sim_ms_clock = 100u;
static int      g_last_itic    = 0;
static uint32_t g_last_frame_ms = 0u;
static uint32_t g_fps_win_start = 0u;
static uint32_t g_fps_win_cnt   = 0u;
static int      g_cur_wpn_slot  = 1;
static uint8_t  g_atexit_reg    = 0u;

static void keyq_push(int pressed, uint8_t key) {
    uint32_t next = (g_keyq_wr + 1u) & (KEYQ_LEN - 1u);
    if (next == g_keyq_rd) return;
    g_keyq[g_keyq_wr].pressed = (uint8_t)(pressed ? 1u : 0u);
    g_keyq[g_keyq_wr].key = key;
    g_keyq_wr = next;
}

static void sync_vk_mask(uint32_t cur_vk) {
    uint32_t diff = cur_vk ^ g_prev_vk_mask;
    for (uint32_t i = 0; i < VK_COUNT; i++) {
        if (diff & (1u << i))
            keyq_push((cur_vk >> i) & 1u, g_vk_doom[i]);
    }
    g_prev_vk_mask = cur_vk;
}

static void cycle_weapon_dir(int dir) {
    for (int step = 0; step < 7; step++) {
        g_cur_wpn_slot += (dir >= 0) ? 1 : -1;
        if (g_cur_wpn_slot > 7) g_cur_wpn_slot = 1;
        if (g_cur_wpn_slot < 1) g_cur_wpn_slot = 7;
        /* Slot 1 = fist/chainsaw, 2 = pistol, 3 = shotgun, 4 = chaingun,
         * 5 = missile, 6 = plasma, 7 = bfg */
        weapontype_t wt = (weapontype_t)(g_cur_wpn_slot - 1);
        if (g_cur_wpn_slot == 1 &&
            (players[consoleplayer].weaponowned[wp_fist] ||
             players[consoleplayer].weaponowned[wp_chainsaw])) {
            break;
        }
        if (wt < NUMWEAPONS && players[consoleplayer].weaponowned[wt])
            break;
    }
    uint8_t wk = (uint8_t)('0' + g_cur_wpn_slot);
    keyq_push(1, wk);
    keyq_push(0, wk);
}

/* Translate simulator single-key dispatch (`arg` when !is_autonomous) into snapshot */
static void apply_sim_arg(uint32_t arg, struct dp2m_input_snapshot *inp) {
    switch (arg) {
    case 0x01u: inp->held_mask |= DP2M_BTN_OK;      break;
    case 0x02u: inp->held_mask |= DP2M_BTN_UP;      break;
    case 0x03u: inp->held_mask |= DP2M_BTN_DOWN;    break;
    case 0x04u: inp->held_mask |= DP2M_BTN_LEFT;    break;
    case 0x05u: inp->held_mask |= DP2M_BTN_RIGHT;   break;
    case 0x06u: inp->held_mask |= DP2M_BTN_MENU;    break;
    case 0x07u: inp->held_mask |= DP2M_BTN_DISP;    break;
    case 0x08u: inp->held_mask |= DP2M_BTN_AEL;     break;
    case 0x09u: inp->held_mask |= DP2M_BTN_QS;      break;
    case 0x0Au: inp->held_mask |= DP2M_BTN_MODE;    break;
    case 0x0Cu: inp->held_mask |= DP2M_BTN_PLAY;    break;
    case 0x20u: inp->dial_delta = -1;               break;
    case 0x40u: inp->dial_delta = +1;               break;
    case 0xFEu: inp->held_mask |= DP2M_BTN_SHUTTER; break;
    default: break;
    }
}

/* Returns 1 if PLAY was pressed to exit DOOM back to DP2M Live View */
static int poll_and_translate_controls(uint32_t sim_arg) {
    struct dp2m_input_snapshot inp;
    memset(&inp, 0, sizeof(inp));
    if (g_dp2m_api->poll_input) {
        g_dp2m_api->poll_input(&inp);
        /* Decode the shutter from the raw word instead of trusting the firmware's bit: firmware built
         * with bit 30 in its mask reports the shutter as held forever, which mutes it for the session. */
        inp.held_mask &= ~DP2M_BTN_SHUTTER;
        if (inp.raw_sw28 & DP2M_SW28_SHUTTER)
            inp.held_mask |= DP2M_BTN_SHUTTER;
    }
    if (!g_dp2m_api->is_autonomous && sim_arg != 0u)
        apply_sim_arg(sim_arg, &inp);
    g_doom_tel.inp = inp;

    uint32_t m = inp.held_mask;
    if (!g_boot_primed) {
        g_boot_held = m;
        g_boot_primed = 1u;
    }
    /* A button inherited from the screen that opened DOOM does nothing until that button is released.
     * Each button is tracked on its own so one stuck line cannot keep the others disabled. */
    g_boot_held &= m;
    m &= ~g_boot_held;
    uint32_t pressed = m & ~g_prev_hw_mask;
    g_prev_hw_mask = m;

    if (pressed & DP2M_BTN_PLAY)
        return 1;

    /* MODE cycles: 0 Dashboard -> 1 Fullscreen 4:3 CRT -> 2 Live HW Explorer */
    if (pressed & DP2M_BTN_MODE) {
        g_doom_tel.hud_mode = (g_doom_tel.hud_mode + 1u) % DP2M_HUD_MODE_COUNT;
    }

    /* QS + AEL chord toggles live accelerometer Tilt-Strafe */
    if ((m & DP2M_BTN_QS) && (pressed & DP2M_BTN_AEL)) {
        g_doom_tel.tilt_strafe ^= 1u;
    }

    boolean in_ui = (menuactive || gamestate != GS_LEVEL);
    uint32_t vk = 0u;

    if (m & DP2M_BTN_QS) {
        /* QS held chord: Left/Right strafe, Dial cycles weapons, Up/Down steps Explorer */
        if (m & DP2M_BTN_LEFT)  vk |= (1u << VK_STRAFE_L);
        if (m & DP2M_BTN_RIGHT) vk |= (1u << VK_STRAFE_R);
        if (g_doom_tel.hud_mode == DP2M_HUD_EXPLORER) {
            if (pressed & DP2M_BTN_UP)
                g_doom_tel.peek_idx = (g_doom_tel.peek_idx + 5u) % 6u;
            if (pressed & DP2M_BTN_DOWN)
                g_doom_tel.peek_idx = (g_doom_tel.peek_idx + 1u) % 6u;
        } else {
            if (m & DP2M_BTN_UP)   { vk |= (1u << VK_UP) | (1u << VK_RUN); }
            if (m & DP2M_BTN_DOWN) { vk |= (1u << VK_DOWN) | (1u << VK_RUN); }
        }
        if (inp.dial_delta != 0)
            cycle_weapon_dir(inp.dial_delta);
    } else {
        if (m & DP2M_BTN_UP)    vk |= (1u << VK_UP);
        if (m & DP2M_BTN_DOWN)  vk |= (1u << VK_DOWN);
        if (m & DP2M_BTN_LEFT)  vk |= (1u << VK_LEFT);
        if (m & DP2M_BTN_RIGHT) vk |= (1u << VK_RIGHT);

        if (inp.dial_delta != 0) {
            if (menuactive) {
                uint8_t mk = (inp.dial_delta > 0) ? KEY_DOWNARROW : KEY_UPARROW;
                keyq_push(1, mk);
                keyq_push(0, mk);
            } else if (gamestate == GS_LEVEL) {
                /* Instantaneous proportional 10-deg/click turn via ev_mouse */
                event_t mev;
                mev.type = ev_mouse;
                mev.data1 = 0;
                mev.data2 = inp.dial_delta * 228;
                mev.data3 = 0;
                D_PostEvent(&mev);
            }
        }
        if (m & DP2M_BTN_AEL)
            vk |= (1u << VK_USE);
    }

    /* Accelerometer Tilt-Strafe when enabled during level play */
    if (g_doom_tel.tilt_strafe && gamestate == GS_LEVEL && !menuactive) {
        if (inp.accel_x < -12)      vk |= (1u << VK_STRAFE_L);
        else if (inp.accel_x > 12)  vk |= (1u << VK_STRAFE_R);
    }

    /* Shutter fires in gameplay and confirms/opens menu on title screen */
    if (m & DP2M_BTN_SHUTTER)
        vk |= (1u << (in_ui ? VK_ENTER : VK_FIRE));

    /* OK confirms in menus/intermissions and fires in gameplay */
    if (m & DP2M_BTN_OK)
        vk |= (1u << (in_ui ? VK_ENTER : VK_FIRE));

    if (m & DP2M_BTN_MENU)
        vk |= (1u << VK_ESCAPE);

    if (m & DP2M_BTN_DISP)
        vk |= (1u << VK_TAB);

    sync_vk_mask(vk);

    /* In non-autonomous simulator mode, release transient key edges right after tic */
    if (!g_dp2m_api->is_autonomous && sim_arg != 0u) {
        g_prev_hw_mask = 0u;
    }
    return 0;
}

/* ---------- doomgeneric Platform Callbacks ---------- */

void DG_Init(void) {
    (void)I_GetTime();
    uint32_t now = DG_GetTicksMs();
    g_last_frame_ms = now;
    g_fps_win_start = now;
    g_fps_win_cnt = 0u;
}

void DG_DrawFrame(void) {
    uint32_t now = DG_GetTicksMs();
    uint32_t dt = now - g_last_frame_ms;
    g_last_frame_ms = now;
    if (dt > 250u) dt = 250u;
    g_doom_tel.frame_ms = dt;
    g_doom_tel.frame_hist[g_doom_tel.hist_pos & 63u] = (uint8_t)dt;
    g_doom_tel.hist_pos++;

    g_fps_win_cnt++;
    uint32_t win = now - g_fps_win_start;
    if (win >= 500u) {
        g_doom_tel.fps = (g_fps_win_cnt * 1000u) / win;
        g_fps_win_start = now;
        g_fps_win_cnt = 0u;
    } else if (g_doom_tel.fps == 0u && dt > 0u) {
        g_doom_tel.fps = 1000u / dt;
    }

    dp2m_video_present_frame();
}

static void dp2m_sleep_to_tic(uint32_t target_tic) {
    if (!g_dp2m_api || !g_dp2m_api->sleep_ms) return;
    uint32_t sec = target_tic / 35u;
    uint32_t rem = target_tic - sec * 35u;
    uint32_t target_ms = sec * 1000u + (rem * 1000u + 34u) / 35u;
    uint32_t elapsed_ms = (uint32_t)I_GetTimeMS();
    if (target_ms > elapsed_ms) {
        uint32_t wait = target_ms - elapsed_ms;
        if (wait > 35u) wait = 29u;
        g_dp2m_api->sleep_ms(wait);
    }
}

void DG_SleepMs(uint32_t ms) {
    if (!g_dp2m_api) return;
    if (g_dp2m_api->is_autonomous) {
        if (ms == 0u || !g_dp2m_api->sleep_ms) return;
        if (ms <= 2u) {
            /* I_Sleep(1) in TryRunTics / wipe_ScreenWipe waits for next 35 Hz tic:
             * sleep directly to the tic boundary in one dly_tsk call. */
            int cur_tic = I_GetTime();
            if (cur_tic >= 0) {
                dp2m_sleep_to_tic((uint32_t)(cur_tic + 1));
                return;
            }
        }
        g_dp2m_api->sleep_ms(ms);
    } else {
        if (ms <= 2u)
            g_sim_ms_clock += 29u;
        else
            g_sim_ms_clock += ms;
    }
}

uint32_t DG_GetTicksMs(void) {
    if (g_dp2m_api && g_dp2m_api->is_autonomous && g_dp2m_api->get_ms)
        return g_dp2m_api->get_ms();
    return g_sim_ms_clock;
}

int vanilla_keyboard_mapping = 1;

void I_InitInput(void) {}

/* Drain all queued key press & release events without upstream i_input.c's
 * premature `break;` after a single ev_keyup. */
void I_GetEvent(void) {
    int pressed = 0;
    unsigned char key = 0;
    while (DG_GetKey(&pressed, &key)) {
        if (key == 0) continue;
        event_t ev;
        ev.type = pressed ? ev_keydown : ev_keyup;
        ev.data1 = (int)key;
        ev.data2 = pressed ? (int)key : 0;
        ev.data3 = 0;
        D_PostEvent(&ev);
    }
}

int DG_GetKey(int *pressed, unsigned char *doomKey) {
    if (g_keyq_rd == g_keyq_wr) return 0;
    *pressed = g_keyq[g_keyq_rd].pressed;
    *doomKey = g_keyq[g_keyq_rd].key;
    g_keyq_rd = (g_keyq_rd + 1u) & (KEYQ_LEN - 1u);
    return 1;
}

void DG_SetWindowTitle(const char *title) { (void)title; }

void doomgeneric_Create(int argc, char **argv) {
    myargc = argc;
    myargv = argv;
    M_FindResponseFile();
    DG_Init();
    D_DoomMain();
}

static void dp2m_atexit_quit(void) {
    if (g_doom_jmp_armed)
        dp2m_longjmp(g_doom_exit_jmp, 1);
}

static const char *probe_sd_iwad(void) {
    static const char *const cands[] = {
        "DOOM1.WAD",
        "DOOM.WAD",
        "DOOMU.WAD",
        "DOOM2.WAD",
        "PLUTONIA.WAD",
        "TNT.WAD",
        "FREEDOOM1.WAD",
        "FREEDOOM2.WAD",
    };
    for (size_t i = 0; i < sizeof(cands) / sizeof(cands[0]); i++) {
        int32_t fd = dp2m_sd_open_read(cands[i]);
        if (fd >= 0) {
            dp2m_sd_close(fd);
            return cands[i];
        }
    }
    return NULL;
}

static void dp2m_cleanup_all(void) {
    g_doom_jmp_armed = 0;
    dp2m_close_all_files();
    dp2m_free_all();
}

static int32_t dp2m_doom_entry_body(const struct dp2m_app_api *api, uint32_t cmd, uint32_t arg) {
    if (cmd == DP2M_APP_INIT)
        g_app_stack[0] = DP2M_STACK_GUARD_MAGIC;
    if (!api || api->abi_ver != DP2M_APP_ABI_VER)
        return -1;
    g_dp2m_api = api;

    if (cmd == DP2M_APP_INIT) {
        dp2m_libc_reset();
        memset(&g_doom_tel, 0, sizeof(g_doom_tel));
        __asm__ volatile("mov %0, sp" : "=r"(g_doom_tel.entry_sp));
        g_keyq_rd = g_keyq_wr = 0u;
        g_prev_vk_mask = 0u;
        g_prev_hw_mask = 0xFFFFFFFFu;
        g_boot_held = 0u;
        g_boot_primed = 0u;
        g_sim_ms_clock = 100u;
        g_cur_wpn_slot = 2;

        const char *iwad = probe_sd_iwad();
        if (!iwad) {
            dp2m_cleanup_all();
            return -1;
        }

        dp2m_draw_boot_progress("PROBING SD IWAD & ZONE", 0u, 100u);

        static char iwad_arg[24];
        strncpy(iwad_arg, iwad, sizeof(iwad_arg) - 1u);
        iwad_arg[sizeof(iwad_arg) - 1u] = '\0';

        static char *argv_std[] = {
            "doom", "-iwad", iwad_arg, "-mb", "4", "-nogui", NULL
        };
        static char *argv_warp[] = {
            "doom", "-iwad", iwad_arg, "-mb", "4", "-nogui", "-warp", "1", "1", NULL
        };
        uint32_t hdr_flags = *(const volatile uint32_t *)&g_dp2m_hdr.flags;
        int use_warp = ((hdr_flags & 1u) != 0u) || (arg == 1u);
        char **argv = use_warp ? argv_warp : argv_std;
        int argc = use_warp ? 9 : 6;

        g_doom_jmp_armed = 1;
        if (dp2m_setjmp(g_doom_exit_jmp) != 0) {
            dp2m_cleanup_all();
            return -1;
        }

        if (!g_atexit_reg) {
            I_AtExit(dp2m_atexit_quit, false);
            g_atexit_reg = 1u;
        }
        doomgeneric_Create(argc, argv);
        g_last_itic = I_GetTime();
        g_doom_jmp_armed = 0;
        return 0;
    }

    if (cmd == DP2M_APP_STEP) {
        /* Exact 35.000 Hz tic-boundary pacing (ceil(next_tic * 1000 / 35) ms):
         * sleeps at most once per frame so TryRunTics() never spins I_Sleep(1). */
        if (api->is_autonomous) {
            int cur_tic = I_GetTime();
            if (cur_tic <= g_last_itic)
                dp2m_sleep_to_tic((uint32_t)(g_last_itic + 1));
        } else {
            g_sim_ms_clock += 29u;
        }

        if (poll_and_translate_controls(arg) != 0)
            return 1;

        g_doom_jmp_armed = 1;
        if (dp2m_setjmp(g_doom_exit_jmp) != 0) {
            g_doom_jmp_armed = 0;
            return 1;
        }
        doomgeneric_Tick();
        g_last_itic = I_GetTime();
        g_doom_jmp_armed = 0;

        /* Release any simulator one-shot keys after the tic has processed them */
        if (!api->is_autonomous && arg != 0u && g_prev_vk_mask != 0u) {
            sync_vk_mask(0u);
        }
        return 0;
    }

    if (cmd == DP2M_APP_SHUTDOWN) {
        g_doom_jmp_armed = 1;
        if (dp2m_setjmp(g_doom_exit_jmp) == 0) {
            dp2m_cleanup_all();
        } else {
            g_doom_jmp_armed = 0;
        }
        return 0;
    }

    return -1;
}
