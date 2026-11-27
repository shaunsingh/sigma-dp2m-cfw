/* Bare-metal ARM926EJ-S Big-Endian DOOM (E1M1 Hangar) shared definitions */
#ifndef DOOM_H
#define DOOM_H

#include "dp2m.h"
#include "catalog.h"

#define D_W      320
#define D_H      240
#define D_VIEWH  200
#define D_HALF_H 100
#define D_HALF_W 160
#define D_PROJ   160 /* 90 deg horizontal FOV: focal length = D_HALF_W */

#define ANG_MASK 1023u
#define ANG_90   256
#define ANG_180  512
#define ANG_270  768

/* Texture IDs (64x64 procedural 8bpp indexed) */
enum {
    TEX_STARTAN = 0,
    TEX_TECH    = 1,
    TEX_BRICK   = 2,
    TEX_DOOR    = 3,
    TEX_EXIT    = 4,
    TEX_STEP    = 5,
    FLAT_BLUE   = 6,
    FLAT_FLOOR1 = 7,
    FLAT_CEIL1  = 8,
    FLAT_NUKAGE = 9,
    FLAT_GRASS  = 10,
    FLAT_BRIDGE = 11,
    TEX_COUNT   = 12,
    TEX_SKY     = 255
};

/* Sector flags */
#define SEC_BLINK   1u
#define SEC_NUKAGE  2u
#define SEC_DOOR    4u

/* Linedef flags */
#define LINE_EXIT   1u
#define LINE_DOOR   2u

/* Thing kinds */
enum {
    TH_NONE    = 0,
    TH_ZOMBIE  = 1,
    TH_IMP     = 2,
    TH_DEMON   = 3,
    TH_MEDIKIT = 4,
    TH_AMMO    = 5,
    TH_ARMOR   = 6,
    TH_FIREBALL = 7
};

/* Weapons */
enum {
    WPN_PISTOL   = 0,
    WPN_SHOTGUN  = 1,
    WPN_CHAINGUN = 2
};

#define MAX_SECS   14
#define MAX_LINES  72
#define MAX_THINGS 16

struct d_line {
    int16_t x1, y1, x2, y2;
    int8_t  back;     /* -1 = solid wall, >=0 = neighbor sector */
    uint8_t tex;      /* wall texture (mid / upper / lower) */
    uint8_t flags;
    uint8_t pad;
};

struct d_sector_def {
    int16_t floor_h, ceil_h;
    uint8_t floor_tex, ceil_tex;
    uint8_t light;
    uint8_t flags;
    uint8_t first_line;
    uint8_t num_lines;
};

struct d_sector {
    int16_t floor_h, ceil_h;
    int16_t base_ceil;
    uint8_t floor_tex, ceil_tex;
    uint8_t light;
    uint8_t flags;
    uint8_t first_line;
    uint8_t num_lines;
    uint8_t door_state; /* 0 closed, 1 opening, 2 open, 3 closing */
    uint8_t door_timer;
};

struct d_thing {
    int32_t x, y, z;   /* 16.16 fixed point (z in map units) */
    int32_t vx, vy;    /* velocity for fireballs */
    int16_t hp;
    uint8_t kind;
    uint8_t sector;
    uint8_t state;     /* 0 idle, 1 chase, 2 pain, 3 dead */
    uint8_t timer;
    uint8_t anim;
    uint8_t pad;
};

struct d_state {
    uint32_t magic;
    int32_t px, py, pz;  /* player 16.16 fixed point */
    int32_t pvx, pvy;    /* momentum */
    int32_t view_z;      /* eye height in map units (floor + 41 + bob) */
    uint32_t angle;      /* 0..1023 (0 = +X East, 256 = +Y North) */
    uint8_t sector;
    uint8_t weapon;      /* 0 pistol, 1 shotgun, 2 chaingun */
    uint8_t automap;
    uint8_t won;
    int16_t health;
    int16_t armor;
    int16_t ammo;
    uint8_t kills;
    uint8_t total_enemies;
    uint8_t flash_tics;  /* muzzle flash & light boost */
    uint8_t recoil;      /* weapon kick */
    uint8_t pain_tics;   /* red tint countdown */
    uint8_t bonus_tics;  /* gold pickup tint countdown */
    uint8_t face_expr;   /* 0..2 glance, 3 grin, 4 ouch */
    uint8_t face_timer;
    uint32_t tic;
    uint32_t rng;
    struct d_sector secs[MAX_SECS];
    struct d_thing things[MAX_THINGS];
};

/* Shared RAM buffers (in .bss) */
extern struct d_state doom_st;
extern uint8_t doom_fb[D_W * D_H];
extern uint8_t doom_tex[TEX_COUNT][64 * 64];
extern uint8_t doom_spr[7][32 * 32];
extern uint8_t doom_cmap[32][256];
extern uint32_t doom_pal32[3][256]; /* 0=normal, 1=pain red, 2=bonus gold */
extern int16_t doom_sin[1024];      /* Q14: -16384..+16384 */

/* Read-only map tables (in DOOM_C) */
extern const struct d_sector_def doom_map_secs[];
extern const struct d_line doom_map_lines[];
extern const uint8_t doom_num_secs;

/* ARM renderer entry points (src/m6m/doom_render.c) */
void doom_render_3d(void);
void doom_upscale_2x2(const uint8_t *src, uint16_t *dst, const uint32_t *pal32);

/* Fast division helpers (no libgcc needed) */
int32_t doom_sdiv(int32_t num, int32_t den);
uint32_t doom_udiv(uint32_t num, uint32_t den);

/* Helpers */
static inline int32_t d_sin(uint32_t a) { return doom_sin[a & ANG_MASK]; }
static inline int32_t d_cos(uint32_t a) { return doom_sin[(a + ANG_90) & ANG_MASK]; }

#endif
