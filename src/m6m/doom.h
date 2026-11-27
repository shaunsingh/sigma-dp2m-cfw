/* Bare-metal ARM926EJ-S Big-Endian DOOM (E1M1 Hangar) shared definitions */
#ifndef DOOM_H
#define DOOM_H

#include "dp2m.h"
#include "catalog.h"

#define D_W        320
#define D_H        240
#define D_VIEWH    200
#define D_HALF_H   100
#define D_HALF_W   160
#define D_PROJ     160 /* 90 deg horizontal FOV: focal length = D_HALF_W */

#define ANG_MASK   1023u
#define ANG_90     256

#define DOOM_SECS  13u
#define DOOM_LINES 66u
#define MAX_THINGS 16

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
#define SEC_BLINK  1u
#define SEC_NUKAGE 2u
#define SEC_DOOR   4u

/* Linedef flags */
#define LINE_EXIT  1u
#define LINE_DOOR  2u

/* Thing kinds */
enum {
    TH_NONE     = 0,
    TH_ZOMBIE   = 1,
    TH_IMP      = 2,
    TH_DEMON    = 3,
    TH_MEDIKIT  = 4,
    TH_AMMO     = 5,
    TH_ARMOR    = 6,
    TH_FIREBALL = 7
};

/* Weapons */
enum {
    WPN_PISTOL   = 0,
    WPN_SHOTGUN  = 1,
    WPN_CHAINGUN = 2
};

struct d_line {
    int16_t x1, y1, x2, y2;
    int8_t  back;  /* -1 = solid wall, >=0 = neighbor sector */
    uint8_t tex;
    uint8_t flags;
};

struct d_sector_def {
    int16_t floor_h, ceil_h;
    uint8_t floor_tex, ceil_tex;
    uint8_t light;
    uint8_t flags;
    uint8_t first_line;
    uint8_t num_lines;
};

struct d_sector_rt {
    int16_t ceil_h;
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
};

struct d_state {
    uint32_t magic;
    int32_t px, py;    /* player 16.16 fixed point */
    int32_t pvx, pvy;
    int32_t view_z;    /* eye height in map units */
    uint32_t angle;    /* 0..1023 (0 = +X East, 256 = +Y North) */
    uint8_t sector;
    uint8_t weapon;    /* 0 pistol, 1 shotgun, 2 chaingun */
    uint8_t automap;
    uint8_t won;
    int16_t health;
    int16_t armor;
    int16_t ammo;
    uint8_t kills;
    uint8_t flash_tics;
    uint8_t recoil;
    uint8_t pain_tics;
    uint8_t bonus_tics;
    uint8_t face_expr; /* 0..2 glance, 3 grin, 4 ouch */
    uint8_t face_timer;
    uint32_t tic;
    uint32_t rng;
    struct d_sector_rt secs[DOOM_SECS];
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
extern const struct d_sector_def doom_map_secs[DOOM_SECS];
extern const struct d_line doom_map_lines[DOOM_LINES];

/* ARM renderer entry points (doom_render.c) */
void doom_render_3d(void);
void doom_upscale_2x2(const uint8_t *src, uint16_t *dst, const uint32_t *pal32);

/* CLZ division helpers (no libgcc) */
int32_t doom_sdiv(int32_t num, int32_t den);
uint32_t doom_udiv(uint32_t num, uint32_t den);

/* Fixed-point & geometry helpers */
static inline int32_t d_sin(uint32_t a) { return doom_sin[a & ANG_MASK]; }
static inline int32_t d_cos(uint32_t a) { return doom_sin[(a + ANG_90) & ANG_MASK]; }
static inline int32_t d_abs(int32_t v) { return v < 0 ? -v : v; }
static inline int32_t d_clamp(int32_t v, int32_t lo, int32_t hi) { return v < lo ? lo : v > hi ? hi : v; }
static inline int32_t d_oct_dist(int32_t dx, int32_t dy) {
    int32_t ax = d_abs(dx), ay = d_abs(dy);
    return ax > ay ? ax + (ay >> 1) : ay + (ax >> 1);
}
static inline int32_t d_shade(int32_t light, int32_t dist_shift) {
    return d_clamp(31 - (light >> 3) + dist_shift, 0, 31);
}
/* CCW edge left-hand cross product in Q4 map units (>= 0 means inside) */
static inline int32_t d_edge_cross_q4(const struct d_line *l, int32_t mx, int32_t my) {
    return ((int32_t)l->x2 - l->x1) * (my - ((int32_t)l->y1 << 4))
         - ((int32_t)l->y2 - l->y1) * (mx - ((int32_t)l->x1 << 4));
}

#endif
