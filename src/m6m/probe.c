/* Bare-metal ARM926EJ-S Big-Endian on-camera diagnostic PROBE applet.
 * Opt-in replacement for doom.c/doom_render.c when `:probe true` is set.
 * Read-only by default; no NOR/NVRAM writes; no CP15 MRC from User mode.
 */
#include <stdarg.h>
#include "dp2m.h"
#include "catalog.h"

#define P2B_BLOCKS  16960u   /* 640x424 / 16 px per block (32 B = 8 words) */
#define P2B_RUNS    8u
#define P3_WIN_BASE 0xA0800000u
#define P3_NWIN     248u     /* 0xA08..0xAFF = 248 x 1 MiB */
#define P3_NSAMP    256u     /* 1 word every 4 KiB per 1 MiB window */
#define P6_LEN      0x10000u /* 64 KiB beyond __bss_end */
#define LOG_CAP     12u
#define PROBE_INIT_MAGIC 0x50524F42u /* payload .bss is not zeroed at boot */

/* Linker bounds from src/m6m/m6m.ld */
extern uint8_t __ram_start[];
extern uint8_t __ram_end[];
extern uint8_t __bss_start[];
extern uint8_t __bss_end[];

/* Verified stock ABI structures (big-endian) */
struct t_rmpl {
    uint16_t wtskid;
    uint16_t pad;
    uint32_t frsz;
    uint32_t maxsz;
};

struct t_rmpf {
    uint16_t wtskid;
    uint16_t pad;
    uint32_t fblkcnt;
};

/* Verified stock entry points (ARM: even, Thumb: |1) */
#define FN_GET_TIM       ((int32_t (*)(uint64_t *))0xA00745F0u)                            /* ARM */
#define FN_REF_MPL       ((int32_t (*)(int32_t, struct t_rmpl *))0xA00722A0u)              /* ARM */
#define FN_REF_MPF       ((int32_t (*)(int32_t, struct t_rmpf *))0xA0071A3Cu)              /* ARM */
#define FN_CLK_GET_A     ((uint32_t (*)(void))0xA007B3F4u)                                 /* ARM */
#define FN_CLK_GET_B     ((uint32_t (*)(void))0xA007B438u)                                 /* ARM */
#define FN_PSAVE_INHIBIT ((void (*)(uint32_t))0xA0040541u)                                 /* Thumb */
#define FN_PSAVE_RELEASE ((void (*)(uint32_t))0xA004056Fu)                                 /* Thumb */
/* Shared stock bitmask has no refcount; keep this probe-only bit exclusive. */
#define PSAVE_PROBE_MASK 0x40000000u

/* Retained from doom_render.c in probe builds for a direct portal-upscale benchmark. */
extern void doom_upscale_2x2(const uint8_t *src, uint16_t *dst, const uint32_t *pal32);
#define FN_MDF_OPEN      ((int32_t (*)(const char *, uint32_t, uint32_t))0xA01111D1u)      /* Thumb */
#define FN_MDF_CLOSE     ((int32_t (*)(int32_t))0xA011125Bu)                               /* Thumb */
#define FN_MDF_READ      ((int32_t (*)(int32_t, void *, uint32_t, uint32_t *))0xA01112B7u) /* Thumb */
#define FN_MDF_FSTAT     ((int32_t (*)(int32_t, void *))0xA0111563u)                       /* Thumb */
#define FN_MDF_LSEEK     ((int32_t (*)(int32_t, int32_t, uint32_t, uint32_t *))0xA012EE4Du) /* Thumb */

/* Colors in big-endian RGBA4444 (R4<<12 | G4<<8 | B4<<4 | A4) */
#define COL_BG     0x000Fu
#define COL_HDR_BG 0x125Fu
#define COL_FTR_BG 0x223Fu
#define COL_WHITE  0xFFFFu
#define COL_YELLOW 0xFF2Fu
#define COL_CYAN   0x5FFFu
#define COL_GREEN  0x6F6Fu
#define COL_RED    0xF55Fu
#define COL_GRAY   0xAAAFu

struct evt_rec {
    uint32_t seq;
    uint32_t ms;
    uint16_t msg;
    uint16_t arg;
};

struct p2_res {
    uint32_t ran;
    uint32_t alu_ms, alu_sink;
    uint32_t rd_ms, rd_sink;
    uint32_t wr_ms;
    uint32_t rnd_ms, rnd_sink;
    uint32_t blit_ms;
    uint32_t portal_ms;
    uint32_t clear_ms;
    uint32_t dcache_ms;
    uint32_t commit_ms;
    uint32_t pres_ms;
};

struct p2b_res {
    uint32_t ran;
    uint32_t warm_ms[6];
    uint32_t cold_ms[6];
    uint32_t sink;
};

struct p4_res {
    uint32_t ran;
    int32_t  fd;
    int32_t  fstat_rc;
    uint32_t size_hi, size_lo;
    int32_t  rd16_rc;
    uint32_t got16;
    uint8_t  head16[16];
    uint32_t seq_bytes, seq_ms;
    uint32_t rnd_ok, rnd_ms;
    int32_t  lseek_rc;
    uint32_t lseek_pos;
    int32_t  close_rc;
};

struct p6_res {
    uint32_t ran;
    uint32_t nz_cnt;
    uint32_t nz_off[8];
    uint8_t  nz_val[8];
    uint8_t  head16[16];
};

struct probe_state {
    uint32_t inited;
    uint32_t page; /* 0..PROBE_PAGES-1 */
    uint32_t total_msg;
    uint32_t m4_0, m4_1, m4_2, m4_3, m4_oth;
    uint32_t m5_cnt, m3_cnt, oth_cnt;
    uint32_t log_head;
    struct evt_rec log[LOG_CAP];

    uint32_t last_ms, dt_ms;
    struct p2_res p2;
    struct p2b_res p2b;

    uint32_t p3_pass, p3_all0, p3_chg_wins;
    uint32_t p3_max_run_win, p3_max_run_len;
    uint32_t p3_mpl_ok, p3_mpf_ok;
    struct t_rmpl p3_mpl[10];
    int32_t  p3_mpl_rc[10];
    struct t_rmpf p3_mpf[32];
    int32_t  p3_mpf_rc[32];

    struct p4_res p4;
    struct p6_res p6_boot;
    struct p6_res p6_now;
};

/* All mutable state & scratch buffers live in .bss (RAM 0xA1C00000..0xA1E00000) */
static struct probe_state pst;
static uint32_t p3_snap[P3_NWIN * P3_NSAMP];
static uint16_t p3_zero[P3_NWIN];
static uint16_t p3_chg[P3_NWIN];
static uint32_t p_scratch64k[16384]; /* 64 KiB shared by P2 write bench & P4 SD read */
static uint8_t  p2_blit_src[320 * 240] __attribute__((aligned(4)));
static uint32_t p2_pal32[256];
static uint8_t  p2b_luma[640 * 424] __attribute__((aligned(4)));
static uint32_t p4_stat_buf[0x90]; /* 0x240 B >= 0x230 B */

/* ---------- 5x7 Bitmap Font (MSB bit 4 = left pixel), placed in DOOM_E (.probe.ro) ---------- */
__attribute__((section(".probe.ro")))
static const uint8_t font5x7[][7] = {
    /* 0..9 (0..9) */
    {0x0E, 0x11, 0x13, 0x15, 0x19, 0x11, 0x0E},
    {0x04, 0x0C, 0x04, 0x04, 0x04, 0x04, 0x0E},
    {0x0E, 0x11, 0x01, 0x02, 0x04, 0x08, 0x1F},
    {0x1F, 0x02, 0x04, 0x02, 0x01, 0x11, 0x0E},
    {0x02, 0x06, 0x0A, 0x12, 0x1F, 0x02, 0x02},
    {0x1F, 0x10, 0x1E, 0x01, 0x01, 0x11, 0x0E},
    {0x06, 0x08, 0x10, 0x1E, 0x11, 0x11, 0x0E},
    {0x1F, 0x01, 0x02, 0x04, 0x08, 0x08, 0x08},
    {0x0E, 0x11, 0x11, 0x0E, 0x11, 0x11, 0x0E},
    {0x0E, 0x11, 0x11, 0x0F, 0x01, 0x02, 0x0C},
    /* A..Z (10..35) */
    {0x0E, 0x11, 0x11, 0x1F, 0x11, 0x11, 0x11},
    {0x1E, 0x11, 0x11, 0x1E, 0x11, 0x11, 0x1E},
    {0x0E, 0x11, 0x10, 0x10, 0x10, 0x11, 0x0E},
    {0x1C, 0x12, 0x11, 0x11, 0x11, 0x12, 0x1C},
    {0x1F, 0x10, 0x10, 0x1E, 0x10, 0x10, 0x1F},
    {0x1F, 0x10, 0x10, 0x1E, 0x10, 0x10, 0x10},
    {0x0E, 0x11, 0x10, 0x17, 0x11, 0x11, 0x0F},
    {0x11, 0x11, 0x11, 0x1F, 0x11, 0x11, 0x11},
    {0x0E, 0x04, 0x04, 0x04, 0x04, 0x04, 0x0E},
    {0x07, 0x02, 0x02, 0x02, 0x02, 0x12, 0x0C},
    {0x11, 0x12, 0x14, 0x18, 0x14, 0x12, 0x11},
    {0x10, 0x10, 0x10, 0x10, 0x10, 0x10, 0x1F},
    {0x11, 0x1B, 0x15, 0x15, 0x11, 0x11, 0x11},
    {0x11, 0x19, 0x15, 0x13, 0x11, 0x11, 0x11},
    {0x0E, 0x11, 0x11, 0x11, 0x11, 0x11, 0x0E},
    {0x1E, 0x11, 0x11, 0x1E, 0x10, 0x10, 0x10},
    {0x0E, 0x11, 0x11, 0x11, 0x15, 0x12, 0x0D},
    {0x1E, 0x11, 0x11, 0x1E, 0x14, 0x12, 0x11},
    {0x0F, 0x10, 0x10, 0x0E, 0x01, 0x01, 0x1E},
    {0x1F, 0x04, 0x04, 0x04, 0x04, 0x04, 0x04},
    {0x11, 0x11, 0x11, 0x11, 0x11, 0x11, 0x0E},
    {0x11, 0x11, 0x11, 0x11, 0x11, 0x0A, 0x04},
    {0x11, 0x11, 0x11, 0x15, 0x15, 0x1B, 0x11},
    {0x11, 0x11, 0x0A, 0x04, 0x0A, 0x11, 0x11},
    {0x11, 0x11, 0x0A, 0x04, 0x04, 0x04, 0x04},
    {0x1F, 0x01, 0x02, 0x04, 0x08, 0x10, 0x1F},
    /* Punctuation (36..48) */
    {0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00}, /* 36: ' ' */
    {0x00, 0x04, 0x04, 0x00, 0x04, 0x04, 0x00}, /* 37: ':' */
    {0x01, 0x01, 0x02, 0x04, 0x08, 0x10, 0x10}, /* 38: '/' */
    {0x00, 0x00, 0x00, 0x1F, 0x00, 0x00, 0x00}, /* 39: '-' */
    {0x00, 0x00, 0x00, 0x00, 0x00, 0x0C, 0x0C}, /* 40: '.' */
    {0x00, 0x00, 0x1F, 0x00, 0x1F, 0x00, 0x00}, /* 41: '=' */
    {0x02, 0x04, 0x08, 0x08, 0x08, 0x04, 0x02}, /* 42: '(' */
    {0x08, 0x04, 0x02, 0x02, 0x02, 0x04, 0x08}, /* 43: ')' */
    {0x00, 0x04, 0x04, 0x1F, 0x04, 0x04, 0x00}, /* 44: '+' */
    {0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x1F}, /* 45: '_' */
    {0x00, 0x00, 0x00, 0x00, 0x04, 0x04, 0x08}, /* 46: ',' */
    {0x10, 0x10, 0x08, 0x04, 0x02, 0x01, 0x01}, /* 47: '\\' */
    {0x04, 0x04, 0x04, 0x04, 0x04, 0x00, 0x04}, /* 48: '!' */
};

static const char *const page_titles[] = {
    "P1 INPUT",
    "P2 TIME/CPU",
    "P2B WRITE PATTERNS",
    "P3A MEM POOLS",
    "P3B RAM A08-A47",
    "P3C RAM A48-A87",
    "P3D RAM A88-AC7",
    "P3E RAM AC8-AFF",
    "P4 SD STORAGE",
    "P5 DISPLAY",
    "P6 BOOT RAM"
};
#define PROBE_PAGES ((uint32_t)(sizeof(page_titles) / sizeof(page_titles[0])))

/* ---------- ARM-Mode Low-Level Helpers & Benchmarks (placed in DOOM_A) ---------- */

__attribute__((noinline, target("arm"), section(".probe.a")))
static uint32_t read_cpsr_arm(void) {
    uint32_t v;
    __asm__ volatile("mrs %0, cpsr" : "=r"(v));
    return v;
}

__attribute__((noinline, target("arm"), section(".probe.a")))
static uint32_t udivmod_arm(uint32_t num, uint32_t den, uint32_t *rem_out) {
    if (den == 0u) {
        if (rem_out) *rem_out = 0u;
        return 0u;
    }
    uint32_t q = 0u, r = 0u;
    for (int i = 31; i >= 0; i--) {
        r = (r << 1) | ((num >> (uint32_t)i) & 1u);
        if (r >= den) {
            r -= den;
            q |= (1u << (uint32_t)i);
        }
    }
    if (rem_out) *rem_out = r;
    return q;
}

__attribute__((noinline, target("arm"), section(".probe.a")))
static uint32_t bench_alu_arm(uint32_t iters) {
    uint32_t a = 0x12345678u, b = 0x9ABCDEF0u;
    for (uint32_t i = 0; i < iters; i++) {
        a = (a + b) ^ (a >> 5);
        b = (b + a + i) ^ (b << 3);
        a += b ^ 0x5A5A5A5Au;
    }
    return a ^ b;
}

__attribute__((noinline, target("arm"), section(".probe.a")))
static uint32_t bench_sdram_rd_arm(const volatile uint32_t *base, uint32_t words) {
    uint32_t s0 = 0u, s1 = 0u, s2 = 0u, s3 = 0u;
    const volatile uint32_t *p = base;
    uint32_t n = words >> 3;
    while (n--) {
        s0 += p[0]; s1 += p[1]; s2 += p[2]; s3 += p[3];
        s0 ^= p[4]; s1 ^= p[5]; s2 ^= p[6]; s3 ^= p[7];
        p += 8;
    }
    return s0 ^ s1 ^ s2 ^ s3;
}

__attribute__((noinline, target("arm"), section(".probe.a")))
static void bench_sdram_wr_arm(volatile uint32_t *buf, uint32_t words, uint32_t passes) {
    uint32_t v = 0xA55A0001u;
    for (uint32_t pass = 0; pass < passes; pass++) {
        volatile uint32_t *p = buf;
        uint32_t n = words >> 3;
        while (n--) {
            p[0] = v; p[1] = v; p[2] = v; p[3] = v;
            p[4] = v; p[5] = v; p[6] = v; p[7] = v;
            p += 8;
        }
        v += 0x01010101u;
    }
}

__attribute__((noinline, target("arm"), section(".probe.a")))
static uint32_t bench_sdram_rnd_arm(const volatile uint32_t *base, uint32_t mask_words, uint32_t count) {
    uint32_t rng = 0xDEADBEEFu;
    uint32_t acc = 0u;
    for (uint32_t i = 0; i < count; i++) {
        rng = rng * 1664525u + 1013904223u;
        acc ^= base[(rng >> 6) & mask_words];
    }
    return acc;
}

/* 320x200 8bpp -> 640x400 RGBA4444 2x2 LUT upscale into ARGB_FB (centered at y=40) */
__attribute__((noinline, target("arm"), section(".probe.a")))
static void bench_blit_320x200_arm(const uint8_t *src, uint16_t *dst_fb, const uint32_t *pal32) {
    uint32_t *dst = (uint32_t *)(dst_fb + 40u * 640u);
    for (uint32_t y = 0; y < 200u; y++) {
        const uint8_t *s = src + y * 320u;
        uint32_t *r0 = dst;
        uint32_t *r1 = dst + 320u;
        for (uint32_t x = 0; x < 320u; x += 4u) {
            uint32_t c0 = pal32[s[x]];
            uint32_t c1 = pal32[s[x + 1u]];
            uint32_t c2 = pal32[s[x + 2u]];
            uint32_t c3 = pal32[s[x + 3u]];
            r0[x] = c0; r0[x + 1u] = c1; r0[x + 2u] = c2; r0[x + 3u] = c3;
            r1[x] = c0; r1[x + 1u] = c1; r1[x + 2u] = c2; r1[x + 3u] = c3;
        }
        dst += 640u;
    }
}

/* ---------- P2B Overlay Write-Pattern Kernels (ARM, 640x424 = 16960 x 32B blocks, placed in DOOM_A) ---------- */

/* W0 store-free baseline: 6 insns/blk, same two luma loads as W2-W5. */
__attribute__((noinline, target("arm"), section(".probe.a")))
static uint32_t bench_w0_loads_arm(const uint32_t *src, uint32_t blocks) {
    register uint32_t r0 __asm__("r0") = 0u;
    register uint32_t r1 __asm__("r1") = (uint32_t)(uintptr_t)src;
    register uint32_t r2 __asm__("r2") = blocks;
    __asm__ volatile(
        "1:\n"
        "ldr   r4, [r1], #4\n"
        "ldr   r5, [r1], #12\n"
        "orr   r0, r0, r4\n"
        "orr   r0, r0, r5\n"
        "subs  r2, r2, #1\n"
        "bne   1b\n"
        : "+r"(r0), "+r"(r1), "+r"(r2)
        :
        : "r4", "r5", "cc", "memory");
    return r0;
}

/* W1 dense: 4 insns/blk, 8 word stores/blk (2x 4-word stmia) */
__attribute__((noinline, target("arm"), section(".probe.a")))
static uint32_t bench_w1_dense_arm(uint32_t *dst, uint32_t blocks, uint32_t col) {
    register uint32_t r0 __asm__("r0") = (uint32_t)(uintptr_t)dst;
    register uint32_t r1 __asm__("r1") = blocks;
    register uint32_t r2 __asm__("r2") = col;
    __asm__ volatile(
        "mov   r4, r2\n"
        "mov   r5, r2\n"
        "mov   r6, r2\n"
        "mov   r7, r2\n"
        "1:\n"
        "stmia r0!, {r4-r7}\n"
        "stmia r0!, {r4-r7}\n"
        "subs  r1, r1, #1\n"
        "bne   1b\n"
        : "+r"(r0), "+r"(r1)
        : "r"(r2)
        : "r4", "r5", "r6", "r7", "cc", "memory");
    return r0;
}

/* W2 sparse single: 7 insns/blk, 2 luma loads + test + 1x str (word 0 only, 7 untouched) */
__attribute__((noinline, target("arm"), section(".probe.a")))
static uint32_t bench_w2_sparse_single_arm(uint32_t *dst, const uint32_t *src, uint32_t blocks, uint32_t col) {
    register uint32_t r0 __asm__("r0") = (uint32_t)(uintptr_t)dst;
    register uint32_t r1 __asm__("r1") = (uint32_t)(uintptr_t)src;
    register uint32_t r2 __asm__("r2") = blocks;
    register uint32_t r3 __asm__("r3") = col;
    __asm__ volatile(
        "1:\n"
        "ldr   r4, [r1], #4\n"
        "ldr   r5, [r1], #12\n"
        "tst   r4, #1\n"
        "strne r3, [r0]\n"
        "add   r0, r0, #32\n"
        "subs  r2, r2, #1\n"
        "bne   1b\n"
        "eor   r0, r4, r5\n"
        : "+r"(r0), "+r"(r1), "+r"(r2)
        : "r"(r3)
        : "r4", "r5", "cc", "memory");
    return r0;
}

/* W3 sparse burst: 8 insns/blk, 2 luma loads + test + 2x stmia on every 8th block only */
__attribute__((noinline, target("arm"), section(".probe.a")))
static uint32_t bench_w3_sparse_burst_arm(uint32_t *dst, const uint32_t *src, uint32_t blocks, uint32_t col) {
    register uint32_t r0 __asm__("r0") = (uint32_t)(uintptr_t)dst;
    register uint32_t r1 __asm__("r1") = (uint32_t)(uintptr_t)src;
    register uint32_t r2 __asm__("r2") = blocks;
    register uint32_t r3 __asm__("r3") = col;
    __asm__ volatile(
        "mov     r6, r3\n"
        "mov     r7, r3\n"
        "mov     r8, r3\n"
        "mov     r9, r3\n"
        "1:\n"
        "ldr     r4, [r1], #4\n"
        "ldr     r5, [r1], #12\n"
        "tst     r5, #1\n"
        "stmiane r0!, {r6-r9}\n"
        "stmiane r0!, {r6-r9}\n"
        "addeq   r0, r0, #32\n"
        "subs    r2, r2, #1\n"
        "bne     1b\n"
        "eor     r0, r4, r5\n"
        : "+r"(r0), "+r"(r1), "+r"(r2)
        : "r"(r3)
        : "r4", "r5", "r6", "r7", "r8", "r9", "cc", "memory");
    return r0;
}

/* W4 mixed burst-with-zeros: 9 insns/blk, 2 luma loads + test + 2x stmia (1 colour + 7 zeros) */
__attribute__((noinline, target("arm"), section(".probe.a")))
static uint32_t bench_w4_mixed_burst_arm(uint32_t *dst, const uint32_t *src, uint32_t blocks, uint32_t col) {
    register uint32_t r0 __asm__("r0") = (uint32_t)(uintptr_t)dst;
    register uint32_t r1 __asm__("r1") = (uint32_t)(uintptr_t)src;
    register uint32_t r2 __asm__("r2") = blocks;
    register uint32_t r3 __asm__("r3") = col;
    __asm__ volatile(
        "mov   r7, #0\n"
        "mov   r8, #0\n"
        "mov   r9, #0\n"
        "mov   r10, #0\n"
        "1:\n"
        "ldr   r4, [r1], #4\n"
        "ldr   r5, [r1], #12\n"
        "tst   r4, #1\n"
        "movne r6, r3\n"
        "moveq r6, #0\n"
        "stmia r0!, {r6-r9}\n"
        "stmia r0!, {r7-r10}\n"
        "subs  r2, r2, #1\n"
        "bne   1b\n"
        "eor   r0, r4, r5\n"
        : "+r"(r0), "+r"(r1), "+r"(r2)
        : "r"(r3)
        : "r4", "r5", "r6", "r7", "r8", "r9", "r10", "cc", "memory");
    return r0;
}

/* W5 mixed conditional: 21 insns/blk, 2 luma loads + 8x (tst+strne) with exactly 1 taken */
__attribute__((noinline, target("arm"), section(".probe.a")))
static uint32_t bench_w5_mixed_cond_arm(uint32_t *dst, const uint32_t *src, uint32_t blocks, uint32_t col) {
    register uint32_t r0 __asm__("r0") = (uint32_t)(uintptr_t)dst;
    register uint32_t r1 __asm__("r1") = (uint32_t)(uintptr_t)src;
    register uint32_t r2 __asm__("r2") = blocks;
    register uint32_t r3 __asm__("r3") = col;
    __asm__ volatile(
        "1:\n"
        "ldr   r4, [r1], #4\n"
        "ldr   r5, [r1], #12\n"
        "tst   r4, #0x01\n"
        "strne r3, [r0, #0]\n"
        "tst   r4, #0x02\n"
        "strne r3, [r0, #4]\n"
        "tst   r4, #0x04\n"
        "strne r3, [r0, #8]\n"
        "tst   r4, #0x08\n"
        "strne r3, [r0, #12]\n"
        "tst   r4, #0x10\n"
        "strne r3, [r0, #16]\n"
        "tst   r4, #0x20\n"
        "strne r3, [r0, #20]\n"
        "tst   r4, #0x40\n"
        "strne r3, [r0, #24]\n"
        "tst   r4, #0x80\n"
        "strne r3, [r0, #28]\n"
        "add   r0, r0, #32\n"
        "subs  r2, r2, #1\n"
        "bne   1b\n"
        "eor   r0, r4, r5\n"
        : "+r"(r0), "+r"(r1), "+r"(r2)
        : "r"(r3)
        : "r4", "r5", "cc", "memory");
    return r0;
}

__attribute__((noinline, target("arm"), section(".probe.a")))
static void fb_fill_rect_arm(uint32_t x, uint32_t y, uint32_t w, uint32_t h, uint16_t col) {
    uint32_t c2 = ((uint32_t)col << 16) | col;
    uint32_t *fb32 = (uint32_t *)ARGB_FB;
    uint32_t x2 = x >> 1, w2 = w >> 1;
    for (uint32_t row = y; row < y + h; row++) {
        uint32_t *p = fb32 + row * 320u + x2;
        for (uint32_t i = 0; i < w2; i++) p[i] = c2;
    }
}

/* Draw one 5x7 glyph scaled 2x2 (10x14 px) at even x into ARGB_FB */
__attribute__((noinline, target("arm"), section(".probe.a")))
static void fb_draw_glyph_arm(uint32_t x, uint32_t y, const uint8_t *rows, uint16_t fg, uint16_t bg) {
    uint32_t fg2 = ((uint32_t)fg << 16) | fg;
    uint32_t bg2 = ((uint32_t)bg << 16) | bg;
    uint32_t *fb32 = (uint32_t *)ARGB_FB;
    uint32_t x2 = x >> 1;
    for (uint32_t r = 0; r < 7u; r++) {
        uint8_t bits = rows[r];
        uint32_t *p0 = fb32 + (y + (r << 1)) * 320u + x2;
        uint32_t *p1 = p0 + 320u;
        for (uint32_t c = 0; c < 5u; c++) {
            uint32_t px = (bits & (0x10u >> c)) ? fg2 : bg2;
            p0[c] = px;
            p1[c] = px;
        }
    }
}

/* ---------- Text & Compact printf Renderer (placed in DOOM_D) ---------- */

__attribute__((section(".probe.d")))
static uint32_t glyph_idx(char ch) {
    if (ch >= '0' && ch <= '9') return (uint32_t)(ch - '0');
    if (ch >= 'A' && ch <= 'Z') return 10u + (uint32_t)(ch - 'A');
    if (ch >= 'a' && ch <= 'z') return 10u + (uint32_t)(ch - 'a');
    switch (ch) {
    case ':': return 37u;
    case '/': return 38u;
    case '-': return 39u;
    case '.': return 40u;
    case '=': return 41u;
    case '(': return 42u;
    case ')': return 43u;
    case '+': return 44u;
    case '_': return 45u;
    case ',': return 46u;
    case '\\': return 47u;
    case '!': return 48u;
    default:  return 36u;
    }
}

__attribute__((section(".probe.d")))
static void draw_str(uint32_t col, uint32_t row, const char *s, uint16_t fg, uint16_t bg) {
    uint32_t x = 8u + col * 12u;
    uint32_t y = 8u + row * 18u;
    while (*s && col < 52u) {
        fb_draw_glyph_arm(x, y, font5x7[glyph_idx(*s)], fg, bg);
        x += 12u;
        col++;
        s++;
    }
}

__attribute__((noinline, section(".probe.d")))
static void p_printf(uint32_t col, uint32_t row, uint16_t fg, uint16_t bg, const char *fmt, ...) {
    static const char hx[] = "0123456789ABCDEF";
    char buf[64];
    char *o = buf, *end = buf + 56;
    va_list ap;
    va_start(ap, fmt);
    while (*fmt && o < end) {
        if (*fmt != '%') {
            *o++ = *fmt++;
            continue;
        }
        fmt++;
        uint32_t width = 0u;
        if (*fmt >= '1' && *fmt <= '8') {
            width = (uint32_t)(*fmt - '0');
            fmt++;
        }
        char sp = *fmt++;
        if (sp == 's') {
            const char *s = va_arg(ap, const char *);
            while (s && *s && o < end) *o++ = *s++;
        } else if (sp == 'c') {
            *o++ = (char)va_arg(ap, int);
        } else if (sp == 'X') {
            uint32_t v = va_arg(ap, uint32_t);
            uint32_t n = width ? width : 8u;
            for (int i = (int)n - 1; i >= 0 && o < end; i--)
                *o++ = hx[(v >> ((uint32_t)i << 2)) & 0xFu];
        } else if (sp == 'u' || sp == 'd') {
            uint32_t v;
            if (sp == 'd') {
                int32_t sv = va_arg(ap, int32_t);
                if (sv < 0) {
                    *o++ = '-';
                    v = (uint32_t)(-(sv + 1)) + 1u;
                } else {
                    v = (uint32_t)sv;
                }
            } else {
                v = va_arg(ap, uint32_t);
            }
            char tmp[12];
            uint32_t n = 0u;
            if (v == 0u) tmp[n++] = '0';
            while (v) {
                uint32_t rem = 0u;
                v = udivmod_arm(v, 10u, &rem);
                tmp[n++] = (char)('0' + rem);
            }
            while (n && o < end) *o++ = tmp[--n];
        } else if (sp == '%') {
            *o++ = '%';
        }
    }
    va_end(ap);
    *o = 0;
    draw_str(col, row, buf, fg, bg);
}

__attribute__((noinline, section(".probe.d")))
static uint64_t read_tick_ms(void) {
    uint64_t t = 0u;
    FN_GET_TIM(&t);
    if (t == 0u) {
        uint32_t h1 = REG32(0xA07F3788u);
        uint32_t lo = REG32(0xA07F378Cu);
        uint32_t h2 = REG32(0xA07F3788u);
        if (h1 != h2) lo = REG32(0xA07F378Cu);
        t = ((uint64_t)h2 << 32) | lo;
    }
    return t;
}

__attribute__((noinline, section(".probe.d")))
static void draw_hex_bytes(uint32_t row, const char *label, const uint8_t *b, uint32_t n, uint16_t fg) {
    static const char hx[] = "0123456789ABCDEF";
    char buf[64];
    char *o = buf;
    while (*label) *o++ = *label++;
    for (uint32_t i = 0; i < n; i++) {
        *o++ = hx[b[i] >> 4];
        *o++ = hx[b[i] & 0xFu];
        *o++ = ' ';
    }
    *o = 0;
    draw_str(0, row, buf, fg, COL_BG);
}

/* ---------- Probe Actions (placed in DOOM_E) ---------- */

__attribute__((noinline, section(".probe.e")))
static void probe_p6_survey(struct p6_res *out) {
    uintptr_t start = (uintptr_t)__bss_end;
    uintptr_t limit = (uintptr_t)__ram_end;
    uint32_t len = (limit > start) ? (uint32_t)(limit - start) : 0u;
    if (len > P6_LEN) len = P6_LEN;

    const volatile uint8_t *p = (const volatile uint8_t *)start;
    uint32_t nz = 0u;
    for (uint32_t i = 0; i < 16u; i++)
        out->head16[i] = (i < len) ? p[i] : 0u;

    for (uint32_t i = 0; i < len; i++) {
        uint8_t b = p[i];
        if (b != 0u) {
            if (nz < 8u) {
                out->nz_off[nz] = i;
                out->nz_val[nz] = b;
            }
            nz++;
        }
    }
    out->nz_cnt = nz;
    out->ran = 1u;
}

__attribute__((noinline, section(".probe.e")))
static void probe_p3_pools(void) {
    uint32_t ok_l = 0u, ok_f = 0u;
    for (uint32_t i = 0; i < 10u; i++) {
        pst.p3_mpl[i].frsz = 0u;
        pst.p3_mpl[i].maxsz = 0u;
        int32_t rc = FN_REF_MPL((int32_t)(i + 1u), &pst.p3_mpl[i]);
        pst.p3_mpl_rc[i] = rc;
        if (rc == 0) ok_l++;
    }
    for (uint32_t i = 0; i < 32u; i++) {
        pst.p3_mpf[i].fblkcnt = 0u;
        int32_t rc = FN_REF_MPF((int32_t)(i + 1u), &pst.p3_mpf[i]);
        pst.p3_mpf_rc[i] = rc;
        if (rc == 0) ok_f++;
    }
    pst.p3_mpl_ok = ok_l;
    pst.p3_mpf_ok = ok_f;
}

__attribute__((noinline, section(".probe.e")))
static void probe_p3_scan(void) {
    uint32_t pass = pst.p3_pass;
    uint32_t all0 = 0u, chg_wins = 0u;
    uint32_t cur_run = 0u, cur_start = 0u, best_run = 0u, best_start = 0u;

    for (uint32_t w = 0; w < P3_NWIN; w++) {
        uint32_t base = P3_WIN_BASE + (w << 20);
        uint16_t z = 0u, ch = 0u;
        uint32_t *sn = &p3_snap[w * P3_NSAMP];
        for (uint32_t s = 0; s < P3_NSAMP; s++) {
            uint32_t val = REG32(base + (s << 12));
            if (val == 0u) z++;
            if (pass > 0u && val != sn[s]) ch++;
            sn[s] = val;
        }
        p3_zero[w] = z;
        p3_chg[w] = ch;
        if (z == P3_NSAMP && ch == 0u) {
            all0++;
            if (cur_run == 0u) cur_start = w;
            cur_run++;
            if (cur_run > best_run) {
                best_run = cur_run;
                best_start = cur_start;
            }
        } else {
            cur_run = 0u;
        }
        if (ch > 0u) chg_wins++;
    }
    pst.p3_pass = pass + 1u;
    pst.p3_all0 = all0;
    pst.p3_chg_wins = chg_wins;
    pst.p3_max_run_win = best_start;
    pst.p3_max_run_len = best_run;
    probe_p3_pools();
}

__attribute__((noinline, section(".probe.e")))
static uint32_t probe_time_present10(uint32_t with_sync) {
    uint64_t t0 = read_tick_ms();
    for (uint32_t i = 0; i < 10u; i++) {
        if (with_sync) hw_dcache_sync();
        stock_flush_rect(0, 0, 640, 480);
        stock_argb_commit();
    }
    uint64_t t1 = read_tick_ms();
    return (uint32_t)(t1 - t0);
}

__attribute__((noinline, section(".probe.e")))
static void probe_clear424(void) {
    const uint16_t rect[4] = { 0u, 0u, 640u, 424u }, zero[4] = { 0u, 0u, 0u, 0u };
    hw_dcache_sync();
    stock_fill_rect(rect, zero);
}

__attribute__((noinline, section(".probe.e")))
static void probe_p2_run(void) {
    for (uint32_t i = 0; i < 256u; i++) {
        uint16_t c = (uint16_t)((((i >> 4) & 0xFu) << 12) | ((i & 0xFu) << 8) | (((i ^ 0x5Au) & 0xFu) << 4) | 0xFu);
        p2_pal32[i] = ((uint32_t)c << 16) | c;
    }
    for (uint32_t i = 0; i < 320u * 240u; i++)
        p2_blit_src[i] = (uint8_t)i;

    FN_PSAVE_INHIBIT(PSAVE_PROBE_MASK);

    uint64_t t0, t1;

    t0 = read_tick_ms();
    pst.p2.alu_sink = bench_alu_arm(2000000u);
    t1 = read_tick_ms();
    pst.p2.alu_ms = (uint32_t)(t1 - t0);

    t0 = read_tick_ms();
    pst.p2.rd_sink = bench_sdram_rd_arm((const volatile uint32_t *)0xA0000000u, 4u * 1024u * 1024u);
    t1 = read_tick_ms();
    pst.p2.rd_ms = (uint32_t)(t1 - t0);

    t0 = read_tick_ms();
    bench_sdram_wr_arm((volatile uint32_t *)p_scratch64k, 16384u, 64u);
    t1 = read_tick_ms();
    pst.p2.wr_ms = (uint32_t)(t1 - t0);

    t0 = read_tick_ms();
    pst.p2.rnd_sink = bench_sdram_rnd_arm((const volatile uint32_t *)0xA0000000u, 0x003FFFFFu, 524288u);
    t1 = read_tick_ms();
    pst.p2.rnd_ms = (uint32_t)(t1 - t0);

    t0 = read_tick_ms();
    for (uint32_t f = 0; f < 10u; f++)
        bench_blit_320x200_arm(p2_blit_src, ARGB_FB, p2_pal32);
    t1 = read_tick_ms();
    pst.p2.blit_ms = (uint32_t)(t1 - t0);

    t0 = read_tick_ms();
    for (uint32_t f = 0; f < 10u; f++)
        doom_upscale_2x2(p2_blit_src, ARGB_FB, p2_pal32);
    t1 = read_tick_ms();
    pst.p2.portal_ms = (uint32_t)(t1 - t0);

    t0 = read_tick_ms();
    for (uint32_t f = 0; f < 10u; f++)
        probe_clear424();
    t1 = read_tick_ms();
    pst.p2.clear_ms = (uint32_t)(t1 - t0);

    t0 = read_tick_ms();
    for (uint32_t i = 0; i < 5000u; i++)
        hw_dcache_sync();
    t1 = read_tick_ms();
    pst.p2.dcache_ms = (uint32_t)(t1 - t0);

    pst.p2.commit_ms = 0xFFFFFFFFu;
    pst.p2.pres_ms = 0xFFFFFFFFu;
    for (uint32_t rep = 0; rep < 3u; rep++) {
        if (rep & 1u) {
            uint32_t p = probe_time_present10(1u);
            uint32_t c = probe_time_present10(0u);
            if (p < pst.p2.pres_ms) pst.p2.pres_ms = p;
            if (c < pst.p2.commit_ms) pst.p2.commit_ms = c;
        } else {
            uint32_t c = probe_time_present10(0u);
            uint32_t p = probe_time_present10(1u);
            if (c < pst.p2.commit_ms) pst.p2.commit_ms = c;
            if (p < pst.p2.pres_ms) pst.p2.pres_ms = p;
        }
    }

    FN_PSAVE_RELEASE(PSAVE_PROBE_MASK);
    pst.p2.ran = 1u;
}

__attribute__((noinline, section(".probe.e")))
static uint32_t run_p2b_one(uint32_t pat, uint32_t cold, uint32_t *sink) {
    const uint32_t *src = (const uint32_t *)p2b_luma;
    uint32_t *dst = (uint32_t *)ARGB_FB;
    uint32_t col = 0xF80FF80Fu;
    uint32_t s = 0u;
    if (cold) hw_dcache_sync();
    uint64_t t0 = read_tick_ms();
    for (uint32_t r = 0; r < P2B_RUNS; r++) {
        switch (pat) {
        case 0u: s += bench_w0_loads_arm(src, P2B_BLOCKS); break;
        case 1u: s += bench_w1_dense_arm(dst, P2B_BLOCKS, col); break;
        case 2u: s += bench_w2_sparse_single_arm(dst, src, P2B_BLOCKS, col); break;
        case 3u: s += bench_w3_sparse_burst_arm(dst, src, P2B_BLOCKS, col); break;
        case 4u: s += bench_w4_mixed_burst_arm(dst, src, P2B_BLOCKS, col); break;
        default: s += bench_w5_mixed_cond_arm(dst, src, P2B_BLOCKS, col); break;
        }
    }
    uint64_t t1 = read_tick_ms();
    *sink += s;
    return (uint32_t)(t1 - t0);
}

__attribute__((noinline, section(".probe.e")))
static void probe_p2b_run(void) {
    /* Populate 640x424 luma source: word0 = 1 (exact 1 hit/block), word1 = ((blk & 7) == 0) */
    uint32_t *lp = (uint32_t *)p2b_luma;
    for (uint32_t b = 0; b < P2B_BLOCKS; b++) {
        lp[b * 4u + 0u] = 1u;
        lp[b * 4u + 1u] = ((b & 7u) == 0u) ? 1u : 0u;
        lp[b * 4u + 2u] = 0u;
        lp[b * 4u + 3u] = 0u;
    }

    FN_PSAVE_INHIBIT(PSAVE_PROBE_MASK);
    uint32_t sink = 0u;
    for (uint32_t p = 0; p < 6u; p++) {
        pst.p2b.warm_ms[p] = 0xFFFFFFFFu;
        pst.p2b.cold_ms[p] = 0xFFFFFFFFu;
    }
    for (uint32_t rep = 0; rep < 3u; rep++) {
        for (uint32_t i = 0; i < 6u; i++) {
            uint32_t p = (rep & 1u) ? 5u - i : i;
            if (rep & 1u) {
                uint32_t c = run_p2b_one(p, 1u, &sink);
                uint32_t w = run_p2b_one(p, 0u, &sink);
                if (c < pst.p2b.cold_ms[p]) pst.p2b.cold_ms[p] = c;
                if (w < pst.p2b.warm_ms[p]) pst.p2b.warm_ms[p] = w;
            } else {
                uint32_t w = run_p2b_one(p, 0u, &sink);
                uint32_t c = run_p2b_one(p, 1u, &sink);
                if (w < pst.p2b.warm_ms[p]) pst.p2b.warm_ms[p] = w;
                if (c < pst.p2b.cold_ms[p]) pst.p2b.cold_ms[p] = c;
            }
        }
    }
    FN_PSAVE_RELEASE(PSAVE_PROBE_MASK);

    pst.p2b.sink = sink;
    pst.p2b.ran = 1u;
    /* Clear overlay framebuffer before redrawing page */
    fb_fill_rect_arm(0, 0, 640, 480, COL_BG);
}

__attribute__((noinline, section(".probe.e")))
static void probe_p4_run(void) {
    struct p4_res *r = &pst.p4;
    memset(r, 0, sizeof(*r));
    r->ran = 1u;

    int32_t fd = FN_MDF_OPEN("I:\\DOOM1.WAD", 0u, 0u);
    r->fd = fd;
    if (fd < 0)
        return;

    memset(p4_stat_buf, 0, sizeof(p4_stat_buf));
    r->fstat_rc = FN_MDF_FSTAT(fd, p4_stat_buf);
    if (r->fstat_rc == 0) {
        r->size_hi = p4_stat_buf[0x210u / 4u];
        r->size_lo = p4_stat_buf[0x214u / 4u];
    }

    uint32_t got = 0u;
    hw_dcache_sync();
    r->rd16_rc = FN_MDF_READ(fd, p_scratch64k, 16u, &got);
    hw_dcache_sync();
    r->got16 = got;
    memcpy(r->head16, p_scratch64k, 16u);

    uint32_t pos = 0u;
    r->lseek_rc = FN_MDF_LSEEK(fd, 0, 0u, &pos);
    r->lseek_pos = pos;
    uint64_t t0 = read_tick_ms();
    uint32_t total = 0u;
    for (uint32_t c = 0; c < 16u; c++) {
        got = 0u;
        hw_dcache_sync();
        int32_t rc = FN_MDF_READ(fd, p_scratch64k, 65536u, &got);
        if (rc != 0 || got == 0u) break;
        total += got;
        if (got < 65536u) break;
    }
    uint64_t t1 = read_tick_ms();
    r->seq_bytes = total;
    r->seq_ms = (uint32_t)(t1 - t0);

    uint32_t max_off = (r->size_lo > 4096u) ? (r->size_lo - 4096u) : 0u;
    if (max_off > 0u) {
        uint32_t rng = 0x13579BDFu;
        uint32_t ok = 0u;
        t0 = read_tick_ms();
        for (uint32_t i = 0; i < 16u; i++) {
            rng = rng * 1664525u + 1013904223u;
            uint32_t off = 0u;
            udivmod_arm(rng, max_off, &off);
            off &= ~3u;
            pos = 0u;
            r->lseek_rc = FN_MDF_LSEEK(fd, (int32_t)off, 0u, &pos);
            r->lseek_pos = pos;
            if (r->lseek_rc != 0) break;
            got = 0u;
            hw_dcache_sync();
            if (FN_MDF_READ(fd, p_scratch64k, 4096u, &got) == 0 && got > 0u)
                ok++;
        }
        t1 = read_tick_ms();
        r->rnd_ok = ok;
        r->rnd_ms = (uint32_t)(t1 - t0);
    }

    r->close_rc = FN_MDF_CLOSE(fd);
}

/* ---------- Page Renderers (placed in DOOM_B) ---------- */

static void draw_chrome(void) {
    fb_fill_rect_arm(0, 0, 640, 480, COL_BG);
    fb_fill_rect_arm(0, 0, 640, 28, COL_HDR_BG);
    fb_fill_rect_arm(0, 438, 640, 42, COL_FTR_BG);
    p_printf(0, 0, COL_YELLOW, COL_HDR_BG, "PROBE V3 D2EDE84 P%u/%u %s",
             pst.page + 1u, PROBE_PAGES, page_titles[pst.page]);
    draw_str(0, 24, "L/R,DIAL:PAGE  OK:ACT  MENU/PLAY:EXIT", COL_WHITE, COL_FTR_BG);
}

static void draw_p1_input(void) {
    uint8_t d[6];
    for (uint32_t i = 0; i < 6u; i++) d[i] = REG8(0xA07AF38Cu + i);
    uint64_t ms = read_tick_ms();

    p_printf(0, 2, COL_WHITE, COL_BG, "MSG TOT:%u M5:%u M3:%u OTH:%u",
             pst.total_msg, pst.m5_cnt, pst.m3_cnt, pst.oth_cnt);
    p_printf(0, 3, pst.m4_3 ? COL_GREEN : COL_CYAN, COL_BG,
             "M4 (4,0):%u (4,1):%u (4,2):%u (4,3):%u",
             pst.m4_0, pst.m4_1, pst.m4_2, pst.m4_3);
    p_printf(0, 5, COL_GREEN, COL_BG, "KEY0 A07FAA1C:%8X KEY1:%8X",
             REG32(0xA07FAA1Cu), REG32(0xA07FAA20u));
    p_printf(0, 6, COL_GREEN, COL_BG, "SCAN A07FAB24:%8X SW28:%8X",
             REG32(0xA07FAB24u), REG32(0xA07FAA28u));
    draw_hex_bytes(7, "DIAL A07AF38C: ", d, 6u, COL_GREEN);
    p_printf(0, 8, COL_YELLOW, COL_BG, "TICK:%8X:%8X DT:%uMS",
             (uint32_t)(ms >> 32), (uint32_t)ms, pst.dt_ms);

    draw_str(0, 10, "RECENT EVENTS (NEWEST FIRST) OK=REFRESH", COL_CYAN, COL_BG);
    uint32_t count = pst.total_msg < LOG_CAP ? pst.total_msg : LOG_CAP;
    for (uint32_t i = 0; i < count; i++) {
        uint32_t idx = pst.log_head + LOG_CAP - 1u - i;
        if (idx >= LOG_CAP) idx -= LOG_CAP;
        const struct evt_rec *e = &pst.log[idx];
        p_printf(0, 11u + i, COL_WHITE, COL_BG, "%u MSG=%2X ARG=%4X T=%u",
                 e->seq, (uint32_t)e->msg, (uint32_t)e->arg, e->ms);
    }
}

static void draw_p2_time_cpu(void) {
    uint64_t t_api = 0u;
    int32_t trc = FN_GET_TIM(&t_api);
    uint32_t cpsr = read_cpsr_arm();

    p_printf(0, 2, COL_WHITE, COL_BG, "GET_TIM RC:%d VAL:%8X:%8X",
             trc, (uint32_t)(t_api >> 32), (uint32_t)t_api);
    p_printf(0, 3, COL_YELLOW, COL_BG, "RAW TICK:%8X:%8X DT:%uMS",
             REG32(0xA07F3788u), REG32(0xA07F378Cu), pst.dt_ms);
    p_printf(0, 4, COL_CYAN, COL_BG, "CPSR:%8X MODE:%2X PSAV:%4X/%u",
             cpsr, cpsr & 0x1Fu, REG32(0xA07DD73Cu), REG32(0xA07DD738u));
    p_printf(0, 5, COL_CYAN, COL_BG, "CLK A:%uM B:%uM PLL:%4X/%4X",
             udivmod_arm(FN_CLK_GET_A(), 1000000u, 0),
             udivmod_arm(FN_CLK_GET_B(), 1000000u, 0),
             REG32(0x9000010Cu), REG32(0x90000110u));

    if (!pst.p2.ran) {
        draw_str(0, 8, "PRESS OK TO RUN TIMED BENCHMARKS", COL_YELLOW, COL_BG);
        draw_str(0, 9, "(POWER-SAVE INHIBITED DURING RUN)", COL_GRAY, COL_BG);
        return;
    }

    draw_str(0, 7, "BENCHMARK RESULTS (OK TO RE-RUN):", COL_YELLOW, COL_BG);
    p_printf(0,  9, COL_GREEN, COL_BG, "A) ALU 2M:       %u MS SINK=%8X", pst.p2.alu_ms, pst.p2.alu_sink);
    p_printf(0, 10, COL_GREEN, COL_BG, "B) SDRAM RD 16M: %u MS SINK=%8X", pst.p2.rd_ms, pst.p2.rd_sink);
    p_printf(0, 11, COL_GREEN, COL_BG, "B) SDRAM WR  4M: %u MS (64K X 64)", pst.p2.wr_ms);
    p_printf(0, 12, COL_GREEN, COL_BG, "C) RND RD 512K:  %u MS SINK=%8X", pst.p2.rnd_ms, pst.p2.rnd_sink);
    p_printf(0, 13, COL_GREEN, COL_BG, "D) BLIT 320X200: %u MS (10 FRAMES)", pst.p2.blit_ms);
    p_printf(0, 14, COL_GREEN, COL_BG, "D2) PORTAL 240:  %u MS (10 FRAMES)", pst.p2.portal_ms);
    p_printf(0, 15, COL_GREEN, COL_BG, "D3) CLEAR 424:   %u MS (10 FRAMES)", pst.p2.clear_ms);
    p_printf(0, 17, COL_WHITE, COL_BG, "E1) DCACHE 5000: %u MS (SVC 10)", pst.p2.dcache_ms);
    p_printf(0, 18, COL_WHITE, COL_BG, "E2) COMMIT 10 MIN3: %u MS (FL+ARGB)", pst.p2.commit_ms);
    p_printf(0, 19, COL_WHITE, COL_BG, "E3) PRESENT10 MIN3: %u MS (DC+FL+CMT)", pst.p2.pres_ms);
}

static void draw_p2b_write_patterns(void) {
    static const uint8_t insns[6] = { 6u, 4u, 7u, 8u, 9u, 21u };
    static const uint32_t stores[6] = {
        0u,              /* W0: store-free */
        P2B_BLOCKS * 8u, /* W1: 135,680 word stores / pass */
        P2B_BLOCKS,      /* W2:  16,960 word stores / pass */
        P2B_BLOCKS,      /* W3:  16,960 word stores / pass */
        P2B_BLOCKS * 8u, /* W4: 135,680 word stores / pass */
        P2B_BLOCKS       /* W5:  16,960 word stores / pass */
    };

    draw_str(0, 2, "640X424 ARGB_FB (8 RUNS X MIN3, ARM)", COL_CYAN, COL_BG);
    draw_str(0, 3, "1 BLK=16PX (32B=8W)  PSAV INHIBITED ON OK", COL_GRAY, COL_BG);

    if (!pst.p2b.ran) {
        draw_str(0, 6, "PRESS OK TO RUN W0..W5 (WARM + COLD)", COL_YELLOW, COL_BG);
        draw_str(0, 7, "COLD = RIGHT AFTER HW_DCACHE_SYNC (SVC 10)", COL_GRAY, COL_BG);
        return;
    }

    draw_str(0, 5, "PAT INSN/B STORES/PASS WARM/8P COLD/8P", COL_YELLOW, COL_BG);
    for (uint32_t i = 0; i < 6u; i++) {
        uint32_t row = 6u + i;
        p_printf(0,  row, COL_GREEN, COL_BG, "W%u", i);
        p_printf(5,  row, COL_WHITE, COL_BG, "%u", (uint32_t)insns[i]);
        p_printf(13, row, COL_WHITE, COL_BG, "%u", stores[i]);
        p_printf(25, row, COL_GREEN, COL_BG, "%uMS", pst.p2b.warm_ms[i]);
        p_printf(34, row, COL_CYAN,  COL_BG, "%uMS", pst.p2b.cold_ms[i]);
    }

    draw_str(0, 13, "PATTERNS (MIN OF 3 CELLS, 8 PASSES/CELL):", COL_CYAN, COL_BG);
    draw_str(0, 14, "W0: 2X LUMA LD + OR, NO STORES", COL_WHITE, COL_BG);
    draw_str(0, 15, "W1: DENSE 2X STMIA (8W/BLK)", COL_WHITE, COL_BG);
    draw_str(0, 16, "W2: SPARSE 1X STR + 2X LUMA LD", COL_WHITE, COL_BG);
    draw_str(0, 17, "W3: SPARSE 2X STMIA EVERY 8TH BLK", COL_WHITE, COL_BG);
    draw_str(0, 18, "W4: MIXED 2X STMIA (1 COL + 7 ZERO)", COL_WHITE, COL_BG);
    draw_str(0, 19, "W5: MIXED 8X TST+STRNE (1 TAKEN)", COL_WHITE, COL_BG);
    draw_str(0, 21, "COLD = AFTER HW_DCACHE_SYNC (SVC 10)", COL_GRAY, COL_BG);
    p_printf(0, 22, COL_YELLOW, COL_BG, "SINK:%8X  (OK=RE-RUN)", pst.p2b.sink);
}

static void draw_p3_pools(void) {
    uint32_t rs = (uint32_t)(uintptr_t)__ram_start;
    uint32_t re = (uint32_t)(uintptr_t)__ram_end;
    uint32_t bs = (uint32_t)(uintptr_t)__bss_start;
    uint32_t be = (uint32_t)(uintptr_t)__bss_end;

    p_printf(0, 2, COL_WHITE, COL_BG, "RAM:%8X-%8X BSS_END:%8X", rs, re, be);
    p_printf(0, 3, COL_YELLOW, COL_BG, "BSS:%uKB FREE:%uKB PASS:%u (OK=SCAN)",
             (be - bs) >> 10, (re - be) >> 10, pst.p3_pass);
    if (pst.p3_pass > 0u) {
        p_printf(0, 4, COL_GREEN, COL_BG, "ALL0:%u/248 CHG:%u MAX0:%3X %uM",
                 pst.p3_all0, pst.p3_chg_wins,
                 (P3_WIN_BASE + (pst.p3_max_run_win << 20)) >> 20, pst.p3_max_run_len);
    } else {
        draw_str(0, 4, "PRESS OK FOR PASS 1, AGAIN FOR DIFF", COL_GRAY, COL_BG);
    }

    p_printf(0, 6, COL_CYAN, COL_BG, "UITRON MPL(1..10) OK:%u", pst.p3_mpl_ok);
    for (uint32_t i = 0; i < 10u; i++) {
        uint32_t col = (i & 1u) ? 26u : 0u;
        uint32_t row = 7u + (i >> 1);
        if (pst.p3_mpl_rc[i] == 0) {
            p_printf(col, row, COL_WHITE, COL_BG, "%u:%7X/%7X",
                     i + 1u, pst.p3_mpl[i].frsz, pst.p3_mpl[i].maxsz);
        } else {
            p_printf(col, row, COL_GRAY, COL_BG, "%u:ERR %d", i + 1u, pst.p3_mpl_rc[i]);
        }
    }

    p_printf(0, 13, COL_CYAN, COL_BG, "UITRON MPF(1..32) OK:%u", pst.p3_mpf_ok);
    for (uint32_t i = 0; i < 32u; i++) {
        uint32_t col = (i & 3u) * 13u;
        uint32_t row = 14u + (i >> 2);
        if (pst.p3_mpf_rc[i] == 0)
            p_printf(col, row, COL_WHITE, COL_BG, "%2X:%4X", i + 1u, pst.p3_mpf[i].fblkcnt);
        else
            p_printf(col, row, COL_GRAY, COL_BG, "%2X:----", i + 1u);
    }
}

static void draw_p3_table(uint32_t start_w, uint32_t count) {
    p_printf(0, 2, COL_CYAN, COL_BG, "PASS:%u WIN:ZERO/CHG (000..100) OK=SCAN", pst.p3_pass);
    if (pst.p3_pass == 0u) {
        draw_str(0, 5, "NO SCAN YET - PRESS OK FOR PASS 1", COL_YELLOW, COL_BG);
        return;
    }
    for (uint32_t i = 0; i < count; i++) {
        uint32_t w = start_w + i;
        uint16_t z = p3_zero[w], ch = p3_chg[w];
        uint16_t fg = ch ? COL_RED : (z == P3_NSAMP ? COL_GREEN : COL_WHITE);
        p_printf((i & 3u) * 13u, 4u + (i >> 2), fg, COL_BG, "%3X:%3X/%3X",
                 (P3_WIN_BASE + (w << 20)) >> 20, (uint32_t)z, (uint32_t)ch);
    }
}

static void draw_p4_storage(void) {
    p_printf(0, 2, COL_CYAN, COL_BG, "FILE:I:\\DOOM1.WAD MDF_ROOT:%8X", REG32(0xA07F3D68u));
    if (!pst.p4.ran) {
        draw_str(0, 5, "PRESS OK TO TEST READ-ONLY SD I/O", COL_YELLOW, COL_BG);
        return;
    }
    const struct p4_res *r = &pst.p4;
    p_printf(0, 4, r->fd >= 0 ? COL_GREEN : COL_RED, COL_BG, "OPEN FD:%8X (%d) %s",
             (uint32_t)r->fd, r->fd, r->fd >= 0 ? "OK" : "NO FILE");
    if (r->fd < 0) return;

    p_printf(0, 6, COL_WHITE, COL_BG, "FSTAT:%d SIZE:%u B (HI:%4X)", r->fstat_rc, r->size_lo, r->size_hi);
    char tag[5];
    for (uint32_t i = 0; i < 4u; i++) {
        uint8_t c = r->head16[i];
        tag[i] = (c >= 32u && c < 127u) ? (char)c : '.';
    }
    tag[4] = 0;
    p_printf(0, 8, COL_YELLOW, COL_BG, "READ16:%d GOT:%u TAG:%s", r->rd16_rc, r->got16, tag);
    draw_hex_bytes(9,  "HEX 0..7: ", r->head16, 8u, COL_WHITE);
    draw_hex_bytes(10, "HEX 8..F: ", r->head16 + 8, 8u, COL_WHITE);

    uint32_t kbs = r->seq_ms ? udivmod_arm(r->seq_bytes, r->seq_ms, 0) : 0u;
    p_printf(0, 12, COL_GREEN, COL_BG, "SEQ:%u KB IN %u MS (%u KB/S)", r->seq_bytes >> 10, r->seq_ms, kbs);
    p_printf(0, 14, COL_GREEN, COL_BG, "RND 16X4K: OK=%u/16 IN %u MS", r->rnd_ok, r->rnd_ms);
    p_printf(0, 16, COL_WHITE, COL_BG, "LSEEK:%d POS:%8X CLOSE:%d", r->lseek_rc, r->lseek_pos, r->close_rc);
}

static void draw_p5_display(void) {
    const volatile uint16_t *ds = (const volatile uint16_t *)0xA07DD754u;
    uint32_t osd_buf  = (ds[2] == 1u) ? 0xA2130000u : 0xA2184600u;
    uint32_t argb_buf = (ds[5] == 1u) ? 0xA21D8C00u : 0xA226EC00u;

    draw_str(0, 2, "FW STATE (0xA07DD754, READ-ONLY):", COL_CYAN, COL_BG);
    p_printf(0, 3, COL_WHITE, COL_BG, "OSD  %uX%u SEL:%u BUF:%8X S:640",
             (uint32_t)ds[0], (uint32_t)ds[1], (uint32_t)ds[2], osd_buf);
    p_printf(0, 4, COL_GRAY, COL_BG, "OSD  RECT:%d,%d,%d,%d",
             (int32_t)(int16_t)ds[6], (int32_t)(int16_t)ds[7],
             (int32_t)(int16_t)ds[8], (int32_t)(int16_t)ds[9]);
    p_printf(0, 6, COL_WHITE, COL_BG, "ARGB %uX%u SEL:%u BUF:%8X S:1280",
             (uint32_t)ds[3], (uint32_t)ds[4], (uint32_t)ds[5], argb_buf);
    p_printf(0, 7, COL_GRAY, COL_BG, "ARGB RECT:%d,%d,%d,%d",
             (int32_t)(int16_t)ds[10], (int32_t)(int16_t)ds[11],
             (int32_t)(int16_t)ds[12], (int32_t)(int16_t)ds[13]);

    draw_str(0, 9, "DISP CH0 MMIO (0x600D0000, READ-ONLY):", COL_CYAN, COL_BG);
    p_printf(0, 10, COL_GREEN, COL_BG, "OSD  C8:%8X %uX%u S:%u",
             REG32(0x600D00C8u), (uint32_t)REG16(0x600D00C2u),
             (uint32_t)REG16(0x600D00C4u), (uint32_t)REG16(0x600D00CCu));
    p_printf(0, 11, COL_GREEN, COL_BG, "CLUT 28:%8X BANK:%u F4:%4X",
             REG32(0x600D0028u), REG32(0x600D0028u) >> 31, (uint32_t)REG16(0x600D00F4u));
    p_printf(0, 13, COL_GREEN, COL_BG, "ARGB 310:%8X %uX%u S:%u",
             REG32(0x600D0310u), (uint32_t)REG16(0x600D0304u),
             (uint32_t)REG16(0x600D0308u), (uint32_t)REG16(0x600D031Cu));
    p_printf(0, 15, COL_WHITE, COL_BG, "LV Y/CB/CR:%8X %8X %8X",
             REG32(0x600D0018u), REG32(0x600D001Cu), REG32(0x600D0020u));
    p_printf(0, 17, COL_YELLOW, COL_BG, "LV_HOOK FCNT:%u Y:%8X", lv_planes.fcnt, lv_planes.y);
}

static void draw_p6_boot_ram(void) {
    uint32_t rs = (uint32_t)(uintptr_t)__ram_start;
    uint32_t re = (uint32_t)(uintptr_t)__ram_end;
    uint32_t bs = (uint32_t)(uintptr_t)__bss_start;
    uint32_t be = (uint32_t)(uintptr_t)__bss_end;
    const struct p6_res *r = pst.p6_now.ran ? &pst.p6_now : &pst.p6_boot;

    p_printf(0, 2, COL_WHITE, COL_BG, "RAM:%8X..%8X (%u KB)", rs, re, (re - rs) >> 10);
    p_printf(0, 3, COL_WHITE, COL_BG, "BSS:%8X..%8X (%u B)", bs, be, be - bs);
    p_printf(0, 5, COL_CYAN, COL_BG, "SURVEY 64K FROM %8X (OK=RESCAN)", be);
    p_printf(0, 6, r->nz_cnt ? COL_RED : COL_GREEN, COL_BG, "BOOT NZ:%u/65536 NOW NZ:%u",
             pst.p6_boot.nz_cnt, r->nz_cnt);
    draw_hex_bytes(8, "HEAD0..7: ", r->head16, 8u, COL_GRAY);
    draw_hex_bytes(9, "HEAD8..F: ", r->head16 + 8, 8u, COL_GRAY);

    draw_str(0, 11, "FIRST 8 NON-ZERO BYTES:", COL_CYAN, COL_BG);
    if (r->nz_cnt == 0u) {
        draw_str(0, 12, "ALL 65536 BYTES ARE ZERO", COL_GREEN, COL_BG);
        return;
    }
    uint32_t show = r->nz_cnt < 8u ? r->nz_cnt : 8u;
    for (uint32_t i = 0; i < show; i++) {
        p_printf(0, 12u + i, COL_YELLOW, COL_BG, " +%4X (%8X) = 0X%2X",
                 r->nz_off[i], be + r->nz_off[i], (uint32_t)r->nz_val[i]);
    }
}

static void probe_present(void) {
    uint32_t now = (uint32_t)read_tick_ms();
    pst.dt_ms = now - pst.last_ms;
    pst.last_ms = now;

    draw_chrome();
    switch (pst.page) {
    case 0u: draw_p1_input(); break;
    case 1u: draw_p2_time_cpu(); break;
    case 2u: draw_p2b_write_patterns(); break;
    case 3u: draw_p3_pools(); break;
    case 4u: draw_p3_table(0u, 64u); break;
    case 5u: draw_p3_table(64u, 64u); break;
    case 6u: draw_p3_table(128u, 64u); break;
    case 7u: draw_p3_table(192u, 56u); break;
    case 8u: draw_p4_storage(); break;
    case 9u: draw_p5_display(); break;
    default: draw_p6_boot_ram(); break;
    }

    hw_dcache_sync();
    stock_flush_rect(0, 0, 640, 480);
    stock_argb_commit();
}

static void probe_clear_planes(void) {
    stock_argb_clear();
    stock_argb_commit();
    stock_osd_clear();
    stock_osd_commit();
}

void doom_boot_reset(void) {
    pst.inited = 0u;
}

static void probe_ensure_init(void) {
    if (pst.inited == PROBE_INIT_MAGIC) return;
    memset(&pst, 0, sizeof(pst));
    pst.inited = PROBE_INIT_MAGIC;
    pst.last_ms = (uint32_t)read_tick_ms();
    probe_p6_survey(&pst.p6_boot);
    probe_p3_pools();
}

static void probe_log_event(uint32_t msg, uint32_t arg) {
    pst.total_msg++;
    if (msg == 4u) {
        if (arg == 0u) pst.m4_0++;
        else if (arg == 1u) pst.m4_1++;
        else if (arg == 2u) pst.m4_2++;
        else if (arg == 3u) pst.m4_3++;
        else pst.m4_oth++;
    } else if (msg == 5u) {
        pst.m5_cnt++;
    } else if (msg == 3u) {
        pst.m3_cnt++;
    } else {
        pst.oth_cnt++;
    }

    struct evt_rec *e = &pst.log[pst.log_head];
    e->seq = pst.total_msg;
    e->ms = (uint32_t)read_tick_ms();
    e->msg = (uint16_t)msg;
    e->arg = (uint16_t)arg;
    pst.log_head++;
    if (pst.log_head >= LOG_CAP) pst.log_head = 0u;
}

static void probe_action(void) {
    if (pst.page == 1u) {
        probe_p2_run();
    } else if (pst.page == 2u) {
        probe_p2b_run();
    } else if (pst.page >= 3u && pst.page <= 7u) {
        probe_p3_scan();
    } else if (pst.page == 8u) {
        probe_p4_run();
    } else if (pst.page == 10u) {
        probe_p6_survey(&pst.p6_now);
    }
}

/* ---------- M6M Screen Vtable Handler (Screen 33 = 0x21) & Menu Launcher ---------- */

uint32_t doom_screen(uint32_t msg, uint32_t arg) {
    probe_ensure_init();
    probe_log_event(msg, arg);

    if (msg == 4u) {
        if (arg == 0u || arg == 1u) {
            stock_osd_clear();
            stock_osd_commit();
            probe_present();
        } else if (arg == 3u) {
            probe_present();
        }
        return 0u;
    }

    if (msg == 3u) {
        probe_clear_planes();
        return 0u;
    }

    if (msg == 5u) {
        if (arg == 0x06u || arg == 0x0Cu) {
            probe_clear_planes();
            stock_screen_goto(SCREEN_LV);
            return 0u;
        }
        if (arg == 0x05u || arg == 0x40u) {
            pst.page = (pst.page + 1u >= PROBE_PAGES) ? 0u : pst.page + 1u;
        } else if (arg == 0x04u || arg == 0x20u) {
            pst.page = (pst.page == 0u) ? (PROBE_PAGES - 1u) : pst.page - 1u;
        } else if (arg == 0x01u) {
            probe_action();
        }
        probe_present();
    }
    return 0u;
}

/* Menu row extra handler (w7 in Setup Page 4): opens SCREEN_DOOM (33 = 0x21) */
uint32_t doom_open(void) {
    volatile uint32_t keep = (uint32_t)(uintptr_t)doom_screen;
    (void)keep;
    stock_screen_post(1u, SCREEN_DOOM);
    return 0u;
}
