/* Shared DP2M standalone binary applet ABI ("D2AP" v1, loaded at 0xA1CC0000) */
#ifndef DP2M_APP_H
#define DP2M_APP_H

#include <stddef.h>
#include <stdint.h>

#define DP2M_APP_MAGIC    0x44324150u /* "D2AP" */
#define DP2M_APP_ABI_VER  1u
#define DP2M_APP_LOAD_VA  0xA1CC0000u
#define DP2M_APP_LIMIT_VA 0xA1E00000u

enum {
    DP2M_APP_INIT     = 0u, /* init engine + open WAD + draw frame 0; ret 0 ok, <0 fallback */
    DP2M_APP_STEP     = 1u, /* step 1 frame (arg = sim key or 0); ret 0 continue, 1 exit */
    DP2M_APP_SHUTDOWN = 2u, /* close files + release all heap blocks */
};

/* Held button bitmask in dp2m_input_snapshot.held_mask */
#define DP2M_BTN_OK      (1u << 0)
#define DP2M_BTN_UP      (1u << 1)
#define DP2M_BTN_DOWN    (1u << 2)
#define DP2M_BTN_LEFT    (1u << 3)
#define DP2M_BTN_RIGHT   (1u << 4)
#define DP2M_BTN_MENU    (1u << 5)
#define DP2M_BTN_DISP    (1u << 6)
#define DP2M_BTN_AEL     (1u << 7)
#define DP2M_BTN_QS      (1u << 8)
#define DP2M_BTN_MODE    (1u << 9)
#define DP2M_BTN_POWER   (1u << 10)
#define DP2M_BTN_PLAY    (1u << 11)
#define DP2M_BTN_SHUTTER (1u << 12)

/* Shutter release switches, from the stock release task (0xA001FEB0). It arms interrupts 0x27 (S1,
 * half-press) and 0x28 (S2, full press); their handlers (0xA001FB70/0xA001FB94, 0xA001FBF0/0xA001FC0E)
 * hold DP2M_RLS_STATE[0] (S1) and [1] (S2) at 1 while the switch is closed. The task picks the edge
 * to arm from bits 3 (S1) and 2 (S2) of DP2M_RLS_PORT, which read 0 while the switch is closed. */
#define DP2M_RLS_STATE   0xA07AF398u
#define DP2M_RLS_PORT    0x90001018u
#define DP2M_RLS_PORT_S1 0x8u
#define DP2M_RLS_PORT_S2 0x4u

/* dp2m_release_sources() bits: two independent readings of "S1 or S2 is closed" */
#define DP2M_RLS_SRC_STATE 1u
#define DP2M_RLS_SRC_LINES 2u

static inline uint32_t dp2m_release_sources(void) {
    const volatile uint8_t *st = (const volatile uint8_t *)(uintptr_t)DP2M_RLS_STATE;
    uint32_t lines = *(const volatile uint32_t *)(uintptr_t)DP2M_RLS_PORT & (DP2M_RLS_PORT_S1 | DP2M_RLS_PORT_S2);
    uint32_t src = 0u;
    if ((st[0] | st[1]) != 0u)
        src |= DP2M_RLS_SRC_STATE;
    if (lines != (DP2M_RLS_PORT_S1 | DP2M_RLS_PORT_S2))
        src |= DP2M_RLS_SRC_LINES;
    return src;
}

struct dp2m_app_hdr {
    uint32_t magic;      /* DP2M_APP_MAGIC */
    uint32_t abi_ver;    /* DP2M_APP_ABI_VER */
    uint32_t load_va;    /* DP2M_APP_LOAD_VA (0xA1CC0000) */
    uint32_t entry_va;   /* entrypoint VA (bit 0 = 1 for Thumb, 0 for ARM) */
    uint32_t image_size; /* bytes in DOOM.BIN loaded at load_va */
    uint32_t bss_end;    /* end of .bss in [load_va + image_size .. DP2M_APP_LIMIT_VA] */
    uint32_t flags;      /* reserved (0) */
    uint32_t reserved;   /* reserved (0) */
};

struct dp2m_input_snapshot {
    uint32_t held_mask;  /* DP2M_BTN_* currently held */
    int32_t  dial_delta; /* signed command dial detents since previous poll (+CW / -CCW) */
    int32_t  accel_x;    /* signed body roll X */
    int32_t  accel_y;    /* signed body pitch Y */
    uint32_t raw_key0;   /* REG32(0xA07FAA1C) */
    uint32_t raw_sw28;   /* REG32(0xA07FAA28): detect-line word, not the shutter */
    uint32_t raw_dial;   /* REG8(0xA07AF38C) */
};

struct dp2m_app_api {
    uint32_t abi_ver;
    uint32_t is_autonomous; /* 1 = real hardware UI task, 0 = simulator event-driven */
    uint16_t *argb_fb;      /* 0xA21D8C00 (640x480 BE RGBA4444) */
    void (*present_argb)(void);
    void (*upscale_2x2)(const uint8_t *src, uint16_t *dst, const uint32_t *pal32);
    uint32_t (*get_ms)(void);
    void (*sleep_ms)(uint32_t ms);
    void (*poll_input)(struct dp2m_input_snapshot *out);
    void *(*malloc_fn)(size_t sz);
    void *(*calloc_fn)(size_t n, size_t sz);
    void *(*realloc_fn)(void *p, size_t sz);
    void (*free_fn)(void *p);
    int32_t (*mdf_open)(const char *path, uint32_t flags, uint32_t mode);
    int32_t (*mdf_read)(int32_t fd, void *buf, uint32_t len, uint32_t *got);
    int32_t (*mdf_lseek)(int32_t fd, int32_t off, uint32_t whence, uint32_t *pos);
    int32_t (*mdf_fstat)(int32_t fd, void *stat_buf);
    int32_t (*mdf_close)(int32_t fd);
};

typedef int32_t (*dp2m_app_entry_fn)(const struct dp2m_app_api *api, uint32_t cmd, uint32_t arg);

#endif
