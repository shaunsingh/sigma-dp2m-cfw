/* Bare-metal ARM926EJ-S Big-Endian DOOM 2.5D Sector/Portal Renderer & 2x2 stmia Upscaler (-marm -O2) */
#include "doom.h"

/* Portal clip arrays & visplane buffers in .bss */
static int16_t ytop_buf[D_W];
static int16_t ybot_buf[D_W];
static int32_t zbuf[D_W]; /* depth (Q4 map units) of nearest solid wall per column for sprite clipping */
static int16_t span_start[D_VIEWH];
static int16_t ceil_y0[D_W], ceil_y1[D_W];
static int16_t flr_y0[D_W], flr_y1[D_W];

/* Fast ARM CLZ-accelerated unsigned 32-bit division (zero libgcc dependency) */
__attribute__((noinline, target("arm"), section(".doom.extra")))
uint32_t doom_udiv(uint32_t n, uint32_t d) {
    if (d <= 1u) return d ? n : 0x7FFFFFFFu;
    if (n < d) return 0u;
    uint32_t shift = (uint32_t)__builtin_clz(d) - (uint32_t)__builtin_clz(n);
    d <<= shift;
    uint32_t q = 0u;
    for (uint32_t i = shift + 1u; i; i--) {
        q <<= 1;
        if (n >= d) {
            n -= d;
            q |= 1u;
        }
        d >>= 1;
    }
    return q;
}

__attribute__((noinline, target("arm"), section(".doom.extra")))
int32_t doom_sdiv(int32_t n, int32_t d) {
    if (d == 0) return n >= 0 ? 0x3FFFFFFF : -0x3FFFFFFF;
    uint32_t neg = (uint32_t)(n ^ d) >> 31;
    uint32_t an = n < 0 ? (uint32_t)(-n) : (uint32_t)n;
    uint32_t ad = d < 0 ? (uint32_t)(-d) : (uint32_t)d;
    uint32_t q = doom_udiv(an, ad);
    return neg ? -(int32_t)q : (int32_t)q;
}

/* Draw a vertical textured wall column (y0..y1 inclusive) with COLORMAP lighting */
__attribute__((noinline, target("arm")))
static void draw_wall_col(int32_t x, int32_t y0, int32_t y1,
                          const uint8_t *tex_col, const uint8_t *cmap,
                          int32_t v, int32_t dv) {
    if (y0 > y1) return;
    uint8_t *dst = doom_fb + y0 * D_W + x;
    int32_t cnt = y1 - y0 + 1;
    while (cnt >= 2) {
        dst[0]   = cmap[tex_col[((uint32_t)v >> 16) & 63u]]; v += dv;
        dst[D_W] = cmap[tex_col[((uint32_t)v >> 16) & 63u]]; v += dv;
        dst += 2 * D_W;
        cnt -= 2;
    }
    if (cnt) *dst = cmap[tex_col[((uint32_t)v >> 16) & 63u]];
}

/* Draw scrolling Phobos sky + mountain silhouette column */
__attribute__((noinline, section(".doom.extra")))
static void draw_sky_col(int32_t x, int32_t y0, int32_t y1, uint32_t sky_x) {
    if (y0 > y1) return;
    /* Procedural mountain skyline height (56..86) from sum of harmonics */
    uint32_t a1 = (sky_x * 2u) & ANG_MASK;
    uint32_t a2 = (sky_x * 5u + 170u) & ANG_MASK;
    uint32_t a3 = (sky_x * 11u + 410u) & ANG_MASK;
    int32_t mtn = 76 - ((doom_sin[a1] >> 10) + (doom_sin[a2] >> 11) + (doom_sin[a3] >> 12));
    uint8_t *dst = doom_fb + y0 * D_W + x;
    for (int32_t y = y0; y <= y1; y++, dst += D_W) {
        if (y < mtn) {
            /* Moody Phobos sunset sky: steel blue-grey -> dusky orange horizon */
            uint32_t g = (uint32_t)y >> 2;
            uint8_t c;
            if (g < 8u) c = (uint8_t)(192u + g);        /* deep twilight blue-grey */
            else if (g < 15u) c = (uint8_t)(200u + (g - 8u)); /* hazy amber-grey */
            else c = (uint8_t)(208u + ((g - 15u) & 7u));      /* warm horizon glow */
            *dst = c;
        } else {
            /* Rugged dark Phobos mountains with subtle vertical strata */
            int32_t dy = y - mtn;
            uint32_t ridge = ((sky_x ^ (uint32_t)dy) >> 1) & 3u;
            *dst = (uint8_t)(64u + ((dy > 14 ? 14 : dy) >> 1) + (ridge == 0 ? 1u : 0u));
        }
    }
}

/* Draw a perspective-correct horizontal textured flat span (x1..x2 inclusive at row y) */
__attribute__((noinline, target("arm")))
static void draw_flat_span(int32_t y, int32_t x1, int32_t x2,
                           int32_t plane_dh, const uint8_t *flat,
                           int32_t sec_light, int32_t cos_a, int32_t sin_a,
                           int32_t px, int32_t py, uint32_t scroll) {
    int32_t dy = y - D_HALF_H;
    if (dy == 0) dy = 1;
    int32_t ady = dy < 0 ? -dy : dy;
    int32_t adh = plane_dh < 0 ? -plane_dh : plane_dh;
    if (adh < 1) adh = 1;

    /* Single division per span: step_scale = (adh << 12) / ady, dist_q12 = step_scale * D_PROJ */
    uint32_t step_scale = doom_udiv((uint32_t)adh << 12, (uint32_t)ady);
    uint32_t dist_q12 = step_scale * (uint32_t)D_PROJ;
    /* Light level with smooth Doom distance diminishing */
    int32_t shade = 31 - (sec_light >> 3) + (int32_t)(dist_q12 >> 18);
    if (shade < 0) shade = 0;
    if (shade > 31) shade = 31;
    const uint8_t *cmap = doom_cmap[shade];

    int32_t d_q2 = (int32_t)(dist_q12 >> 10);
    int32_t fwd_x = d_q2 * cos_a; /* Q16 */
    int32_t fwd_y = d_q2 * sin_a;
    int32_t du = ((int32_t)step_scale * sin_a) >> 10;   /* Q16 */
    int32_t dv = -((int32_t)step_scale * cos_a) >> 10;  /* Q16 */

    int32_t sx = x1 - D_HALF_W;
    uint32_t u = (uint32_t)(px + fwd_x + sx * du) + scroll;
    uint32_t v = (uint32_t)(py + fwd_y + sx * dv) + (scroll >> 1);

    uint8_t *dst = doom_fb + y * D_W + x1;
    int32_t cnt = x2 - x1 + 1;
    while (cnt > 0 && ((uintptr_t)dst & 3u)) {
        *dst++ = cmap[flat[((v >> 10) & 0xFC0u) | ((u >> 16) & 63u)]];
        u += (uint32_t)du;
        v += (uint32_t)dv;
        cnt--;
    }
    while (cnt >= 4) {
        uint32_t p0 = cmap[flat[((v >> 10) & 0xFC0u) | ((u >> 16) & 63u)]]; u += (uint32_t)du; v += (uint32_t)dv;
        uint32_t p1 = cmap[flat[((v >> 10) & 0xFC0u) | ((u >> 16) & 63u)]]; u += (uint32_t)du; v += (uint32_t)dv;
        uint32_t p2 = cmap[flat[((v >> 10) & 0xFC0u) | ((u >> 16) & 63u)]]; u += (uint32_t)du; v += (uint32_t)dv;
        uint32_t p3 = cmap[flat[((v >> 10) & 0xFC0u) | ((u >> 16) & 63u)]]; u += (uint32_t)du; v += (uint32_t)dv;
        *(uint32_t *)(void *)dst = (p0 << 24) | (p1 << 16) | (p2 << 8) | p3;
        dst += 4;
        cnt -= 4;
    }
    while (cnt-- > 0) {
        *dst++ = cmap[flat[((v >> 10) & 0xFC0u) | ((u >> 16) & 63u)]];
        u += (uint32_t)du;
        v += (uint32_t)dv;
    }
}

/* Convert per-column [y0[x]..y1[x]] vertical intervals into horizontal spans (Doom R_MakeSpans!) */
static void emit_visplane(int32_t x_lo, int32_t x_hi,
                          const int16_t *py0, const int16_t *py1,
                          int32_t plane_dh, uint8_t tex_id, int32_t sec_light,
                          int32_t cos_a, int32_t sin_a, int32_t px, int32_t py,
                          uint32_t scroll) {
    if (tex_id == TEX_SKY) {
        uint32_t base_sky = (doom_st.angle << 1);
        for (int32_t x = x_lo; x <= x_hi; x++) {
            if (py0[x] <= py1[x])
                draw_sky_col(x, py0[x], py1[x], base_sky + (uint32_t)x);
        }
        return;
    }
    const uint8_t *flat = doom_tex[tex_id < TEX_COUNT ? tex_id : FLAT_FLOOR1];
    int32_t prev_t = D_VIEWH, prev_b = -1;
    for (int32_t x = x_lo; x <= x_hi + 1; x++) {
        int32_t t = (x <= x_hi) ? py0[x] : D_VIEWH;
        int32_t b = (x <= x_hi) ? py1[x] : -1;
        if (t > b) { t = D_VIEWH; b = -1; }

        /* Close spans that ended before x */
        int32_t stop_t = t < prev_b + 1 ? t : prev_b + 1;
        for (int32_t y = prev_t; y < stop_t; y++)
            draw_flat_span(y, span_start[y], x - 1, plane_dh, flat, sec_light, cos_a, sin_a, px, py, scroll);
        int32_t stop_b = b > prev_t - 1 ? b : prev_t - 1;
        for (int32_t y = stop_b + 1; y <= prev_b; y++)
            draw_flat_span(y, span_start[y], x - 1, plane_dh, flat, sec_light, cos_a, sin_a, px, py, scroll);

        /* Open spans that start at x */
        int32_t open_t = prev_t < b + 1 ? prev_t : b + 1;
        for (int32_t y = t; y < open_t; y++)
            span_start[y] = (int16_t)x;
        int32_t open_b = prev_b > t - 1 ? prev_b : t - 1;
        for (int32_t y = open_b + 1; y <= b; y++)
            span_start[y] = (int16_t)x;

        prev_t = t;
        prev_b = b;
    }
}

/* Portal render queue entry */
struct portal_job {
    uint8_t sec;
    uint8_t depth;
    int16_t x0, x1;
};

#define MAX_JOBS 48
static struct portal_job jobs[MAX_JOBS];
static int16_t job_ytop[MAX_JOBS][D_W];
static int16_t job_ybot[MAX_JOBS][D_W];

/* Compute effective sector light (with blinking and player muzzle flash) */
static int32_t effective_light(const struct d_sector *s) {
    int32_t l = s->light;
    if ((s->flags & SEC_BLINK) && ((doom_st.tic >> 3) & 3u) == 0u)
        l = l > 80 ? l - 80 : 32;
    l += (int32_t)doom_st.flash_tics * 24;
    if (l > 255) l = 255;
    return l;
}

/* Main 2.5D Sector/Portal Renderer */
void doom_render_3d(void) {
    int32_t px = doom_st.px;
    int32_t py = doom_st.py;
    int32_t view_z = doom_st.view_z;
    int32_t cos_a = d_cos(doom_st.angle);
    int32_t sin_a = d_sin(doom_st.angle);
    uint32_t nuk_scroll = doom_st.tic << 15;

    for (int32_t x = 0; x < D_W; x++) {
        zbuf[x] = 0x7FFFFFFF;
    }

    int32_t q_head = 0, q_tail = 1;
    jobs[0].sec = doom_st.sector;
    jobs[0].depth = 0;
    jobs[0].x0 = 0;
    jobs[0].x1 = D_W - 1;
    for (int32_t x = 0; x < D_W; x++) {
        job_ytop[0][x] = 0;
        job_ybot[0][x] = D_VIEWH - 1;
    }

    /* Track which sectors were visited to draw their sprites */
    uint32_t vis_sec_mask = 0u;

    while (q_head < q_tail) {
        int32_t ji = q_head++;
        uint8_t sec_id = jobs[ji].sec;
        uint8_t depth = jobs[ji].depth;
        int32_t win_x0 = jobs[ji].x0;
        int32_t win_x1 = jobs[ji].x1;
        const struct d_sector *sec = &doom_st.secs[sec_id];
        int32_t sec_light = effective_light(sec);
        vis_sec_mask |= 1u << sec_id;

        /* Copy this job's clip window into active arrays and reset visplanes */
        for (int32_t x = win_x0; x <= win_x1; x++) {
            ytop_buf[x] = job_ytop[ji][x];
            ybot_buf[x] = job_ybot[ji][x];
            ceil_y0[x] = D_VIEWH;
            ceil_y1[x] = -1;
            flr_y0[x] = D_VIEWH;
            flr_y1[x] = -1;
        }

        int32_t c_dh = sec->ceil_h - view_z;
        int32_t f_dh = sec->floor_h - view_z;
        int32_t nl = sec->num_lines;
        const struct d_line *lines = &doom_map_lines[sec->first_line];

        for (int32_t li = 0; li < nl; li++) {
            const struct d_line *ln = &lines[li];
            /* Transform endpoints relative to player (in map units Q4 for high precision) */
            int32_t dx1 = ((int32_t)ln->x1 << 4) - (px >> 12);
            int32_t dy1 = ((int32_t)ln->y1 << 4) - (py >> 12);
            int32_t dx2 = ((int32_t)ln->x2 << 4) - (px >> 12);
            int32_t dy2 = ((int32_t)ln->y2 << 4) - (py >> 12);

            /* Camera space: tz = forward depth (Q4), tx = right offset (Q4) */
            /* Forward = (cos_a, sin_a), Right = (sin_a, -cos_a) */
            int32_t tz1 = (dx1 * cos_a + dy1 * sin_a) >> 14;
            int32_t tx1 = (dx1 * sin_a - dy1 * cos_a) >> 14;
            int32_t tz2 = (dx2 * cos_a + dy2 * sin_a) >> 14;
            int32_t tx2 = (dx2 * sin_a - dy2 * cos_a) >> 14;

            /* Both behind near plane? */
            const int32_t NEAR_Z = 32; /* 2.0 map units in Q4 */
            if (tz1 < NEAR_Z && tz2 < NEAR_Z) continue;

            /* Compute wall length for texture U mapping */
            int32_t wdx = (int32_t)ln->x2 - (int32_t)ln->x1;
            int32_t wdy = (int32_t)ln->y2 - (int32_t)ln->y1;
            int32_t awx = wdx < 0 ? -wdx : wdx;
            int32_t awy = wdy < 0 ? -wdy : wdy;
            int32_t wall_len = (awx > awy) ? (awx + (awy >> 1)) : (awy + (awx >> 1));
            int32_t u1 = 0, u2 = wall_len << 4;

            /* Clip against near plane tz = NEAR_Z */
            if (tz1 < NEAR_Z) {
                int32_t num = NEAR_Z - tz1;
                int32_t den = tz2 - tz1;
                tx1 += doom_sdiv(num * (tx2 - tx1), den);
                u1  += doom_sdiv(num * (u2 - u1), den);
                tz1 = NEAR_Z;
            } else if (tz2 < NEAR_Z) {
                int32_t num = NEAR_Z - tz2;
                int32_t den = tz1 - tz2;
                tx2 += doom_sdiv(num * (tx1 - tx2), den);
                u2  += doom_sdiv(num * (u1 - u2), den);
                tz2 = NEAR_Z;
            }

            /* Project to screen X */
            int32_t sx1 = D_HALF_W + doom_sdiv(tx1 * D_PROJ, tz1);
            int32_t sx2 = D_HALF_W + doom_sdiv(tx2 * D_PROJ, tz2);

            /* Back-face cull: CCW sector edges project LEFT-TO-RIGHT (sx1 < sx2) when viewed from inside! */
            /* Wait: verify sign! Player at (0,0) facing +X (cos=1,sin=0). */
            /* East wall goes CCW from (256,0) to (256,256): dx1=256,dy1=0 -> tx1=0; dx2=256,dy2=256 -> tx2=-256 (LEFT)! */
            /* So CCW wall viewed from inside goes RIGHT-TO-LEFT (sx2 < sx1)! Swap so xL=sx2 < xR=sx1! */
            if (sx2 >= sx1) continue;

            int32_t xL = sx2, xR = sx1;
            int32_t zL = tz2, zR = tz1;
            int32_t uL = u2,  uR = u1;

            if (xR < win_x0 || xL > win_x1) continue;

            int32_t cx0 = xL < win_x0 ? win_x0 : xL;
            int32_t cx1 = xR > win_x1 ? win_x1 : xR;
            int32_t span_x = xR - xL;
            if (span_x <= 0) continue;

            /* Precompute projected top & bottom screen Y at endpoints */
            int32_t y1a = D_HALF_H - doom_sdiv(c_dh * (D_PROJ << 4), zL);
            int32_t y1b = D_HALF_H - doom_sdiv(f_dh * (D_PROJ << 4), zL);
            int32_t y2a = D_HALF_H - doom_sdiv(c_dh * (D_PROJ << 4), zR);
            int32_t y2b = D_HALF_H - doom_sdiv(f_dh * (D_PROJ << 4), zR);

            /* Perspective-correct 1/z and u/z endpoints (Q20) */
            int32_t izL = (int32_t)doom_udiv(1u << 24, (uint32_t)zL);
            int32_t izR = (int32_t)doom_udiv(1u << 24, (uint32_t)zR);
            int32_t uzL = doom_sdiv(uL << 12, zL);
            int32_t uzR = doom_sdiv(uR << 12, zR);

            /* Neighbor portal heights if two-sided */
            int32_t back_id = ln->back;
            int32_t by1a = 0, by1b = 0, by2a = 0, by2b = 0;
            int32_t bc_dh = 0, bf_dh = 0;
            int sky_hack = 0;
            if (back_id >= 0) {
                const struct d_sector *bsec = &doom_st.secs[back_id];
                bc_dh = bsec->ceil_h - view_z;
                bf_dh = bsec->floor_h - view_z;
                by1a = D_HALF_H - doom_sdiv(bc_dh * (D_PROJ << 4), zL);
                by1b = D_HALF_H - doom_sdiv(bf_dh * (D_PROJ << 4), zL);
                by2a = D_HALF_H - doom_sdiv(bc_dh * (D_PROJ << 4), zR);
                by2b = D_HALF_H - doom_sdiv(bf_dh * (D_PROJ << 4), zR);
                if (sec->ceil_tex == TEX_SKY && bsec->ceil_tex == TEX_SKY)
                    sky_hack = 1;
            }

            /* Directional contrast (fake contrast like real Doom: N/S slightly brighter, E/W slightly darker) */
            int32_t wall_light = sec_light + ((awx > awy) ? 16 : -16);
            if (wall_light < 0) wall_light = 0;
            if (wall_light > 255) wall_light = 255;

            const uint8_t *tex = doom_tex[ln->tex < TEX_COUNT ? ln->tex : TEX_STARTAN];
            const uint8_t *low_tex = (back_id >= 0 && doom_st.secs[back_id].floor_h > sec->floor_h)
                                         ? doom_tex[TEX_STEP]
                                         : tex;

            /* Portal child job accumulation */
            int32_t p_x0 = D_W, p_x1 = -1;
            int32_t child_ji = -1;
            if (back_id >= 0 && depth < 10 && q_tail < MAX_JOBS) {
                child_ji = q_tail;
            }

            /* Linear interpolants across screen X in 16.16 fixed point */
            int32_t dy_top = doom_sdiv((y2a - y1a) << 16, span_x);
            int32_t dy_bot = doom_sdiv((y2b - y1b) << 16, span_x);
            int32_t diz    = doom_sdiv(izR - izL, span_x);
            int32_t duz    = doom_sdiv(uzR - uzL, span_x);
            int32_t dby_t  = back_id >= 0 ? doom_sdiv((by2a - by1a) << 16, span_x) : 0;
            int32_t dby_b  = back_id >= 0 ? doom_sdiv((by2b - by1b) << 16, span_x) : 0;

            int32_t off = cx0 - xL;
            int32_t cur_yt = (y1a << 16) + off * dy_top;
            int32_t cur_yb = (y1b << 16) + off * dy_bot;
            int32_t cur_iz = izL + off * diz;
            int32_t cur_uz = uzL + off * duz;
            int32_t cur_bt = (by1a << 16) + off * dby_t;
            int32_t cur_bb = (by1b << 16) + off * dby_b;

            for (int32_t x = cx0; x <= cx1; x++) {
                int32_t top = ytop_buf[x];
                int32_t bot = ybot_buf[x];
                if (top <= bot) {
                    int32_t ya = cur_yt >> 16;
                    int32_t yb = cur_yb >> 16;
                    int32_t cya = ya < top ? top : (ya > bot + 1 ? bot + 1 : ya);
                    int32_t cyb = yb < top - 1 ? top - 1 : (yb > bot ? bot : yb);

                    /* Record ceiling visplane above wall */
                    if (cya > top) {
                        ceil_y0[x] = (int16_t)top;
                        ceil_y1[x] = (int16_t)(cya - 1);
                    }
                    /* Record floor visplane below wall */
                    if (cyb < bot) {
                        flr_y0[x] = (int16_t)(cyb + 1);
                        flr_y1[x] = (int16_t)bot;
                    }

                    /* Recover depth z (Q4) and perspective-correct texture U */
                    int32_t iz = cur_iz > 256 ? cur_iz : 256;
                    int32_t z_q4 = (int32_t)doom_udiv(1u << 24, (uint32_t)iz);
                    uint32_t tex_u = ((uint32_t)doom_sdiv(cur_uz, iz >> 8)) & 63u;

                    /* Distance-attenuated COLORMAP lookup */
                    int32_t shade = 31 - (wall_light >> 3) + (z_q4 >> 10);
                    if (shade < 0) shade = 0;
                    if (shade > 31) shade = 31;
                    const uint8_t *cmap = doom_cmap[shade];

                    int32_t wall_h_px = yb - ya;
                    if (wall_h_px < 1) wall_h_px = 1;

                    if (back_id < 0) {
                        /* Solid 1-sided wall */
                        int32_t tex_h = sec->ceil_h - sec->floor_h;
                        int32_t dv = doom_sdiv(tex_h << 16, wall_h_px);
                        int32_t v = (cya - ya) * dv;
                        draw_wall_col(x, cya, cyb, tex + (tex_u << 6), cmap, v, dv);
                        ytop_buf[x] = (int16_t)(bot + 1);
                        if (z_q4 < zbuf[x]) zbuf[x] = z_q4;
                    } else {
                        /* Two-sided portal: draw upper & lower walls and pass middle opening to child */
                        int32_t bya = cur_bt >> 16;
                        int32_t byb = cur_bb >> 16;
                        int32_t cbya = bya < cya ? cya : (bya > bot + 1 ? bot + 1 : bya);
                        int32_t cbyb = byb < top - 1 ? top - 1 : (byb > cyb ? cyb : byb);

                        /* Upper wall (e.g., doorway header or sliding door!) */
                        if (cbya > cya) {
                            if (sky_hack) {
                                /* Sky-to-sky upper wall renders sky instead of brick */
                                if (ceil_y0[x] > ceil_y1[x]) ceil_y0[x] = (int16_t)cya;
                                ceil_y1[x] = (int16_t)(cbya - 1);
                            } else {
                                int32_t tex_h = sec->ceil_h - sec->floor_h;
                                int32_t dv = doom_sdiv(tex_h << 16, wall_h_px);
                                /* Peg door texture to bottom of moving door ceiling so it slides UP! */
                                int32_t v;
                                if (ln->flags & LINE_DOOR) {
                                    v = ((cya - bya) * dv) + (64 << 16);
                                } else {
                                    v = (cya - ya) * dv;
                                }
                                draw_wall_col(x, cya, cbya - 1, tex + (tex_u << 6), cmap, v, dv);
                            }
                        }

                        /* Lower wall (e.g., stair riser or pit drop-off) */
                        if (cyb > cbyb) {
                            int32_t tex_h = sec->ceil_h - sec->floor_h;
                            int32_t dv = doom_sdiv(tex_h << 16, wall_h_px);
                            int32_t v = (cbyb + 1 - byb) * dv;
                            draw_wall_col(x, cbyb + 1, cyb, low_tex + (tex_u << 6), cmap, v, dv);
                        }

                        /* Remaining portal opening */
                        int32_t ntop = cbya > cya ? cbya : cya;
                        int32_t nbot = cbyb < cyb ? cbyb : cyb;
                        if (ntop <= nbot && child_ji >= 0) {
                            if (x < p_x0) p_x0 = x;
                            if (x > p_x1) p_x1 = x;
                            job_ytop[child_ji][x] = (int16_t)ntop;
                            job_ybot[child_ji][x] = (int16_t)nbot;
                        } else {
                            if (child_ji >= 0) {
                                job_ytop[child_ji][x] = D_VIEWH;
                                job_ybot[child_ji][x] = -1;
                            }
                            if (z_q4 < zbuf[x]) {
                                /* Closed door / fully blocked portal occludes sprites behind it! */
                                zbuf[x] = z_q4;
                            }
                        }
                        ytop_buf[x] = (int16_t)(bot + 1);
                    }
                } else if (child_ji >= 0) {
                    job_ytop[child_ji][x] = D_VIEWH;
                    job_ybot[child_ji][x] = -1;
                }
                cur_yt += dy_top;
                cur_yb += dy_bot;
                cur_iz += diz;
                cur_uz += duz;
                cur_bt += dby_t;
                cur_bb += dby_b;
            }

            if (child_ji >= 0 && p_x0 <= p_x1) {
                /* Close any columns in [p_x0..p_x1] that weren't written */
                jobs[child_ji].sec = (uint8_t)back_id;
                jobs[child_ji].depth = (uint8_t)(depth + 1u);
                jobs[child_ji].x0 = (int16_t)p_x0;
                jobs[child_ji].x1 = (int16_t)p_x1;
                q_tail++;
            }
        }

        /* Emit ceiling and floor horizontal textured spans for this sector! */
        uint32_t flr_scroll = (sec->flags & SEC_NUKAGE) ? nuk_scroll : 0u;
        emit_visplane(win_x0, win_x1, ceil_y0, ceil_y1, c_dh, sec->ceil_tex, sec_light, cos_a, sin_a, px, py, 0u);
        emit_visplane(win_x0, win_x1, flr_y0, flr_y1, f_dh, sec->floor_tex, sec_light, cos_a, sin_a, px, py, flr_scroll);
    }

    /* Render billboarded sprites (depth-sorted back-to-front, clipped to zbuf!) */
    int32_t order[MAX_THINGS];
    int32_t dists[MAX_THINGS];
    int32_t n_vis = 0;

    for (int32_t i = 0; i < MAX_THINGS; i++) {
        const struct d_thing *th = &doom_st.things[i];
        if (th->kind == TH_NONE) continue;
        if (!(vis_sec_mask & (1u << th->sector))) continue;
        int32_t dx = (th->x - px) >> 12; /* Q4 */
        int32_t dy = (th->y - py) >> 12;
        int32_t tz = (dx * cos_a + dy * sin_a) >> 14;
        if (tz < 64) continue; /* too close / behind camera */
        order[n_vis] = i;
        dists[n_vis] = tz;
        n_vis++;
    }

    /* Insertion sort far-to-near */
    for (int32_t i = 1; i < n_vis; i++) {
        int32_t oi = order[i], di = dists[i], j = i - 1;
        while (j >= 0 && dists[j] < di) {
            order[j + 1] = order[j];
            dists[j + 1] = dists[j];
            j--;
        }
        order[j + 1] = oi;
        dists[j + 1] = di;
    }

    for (int32_t vi = 0; vi < n_vis; vi++) {
        const struct d_thing *th = &doom_st.things[order[vi]];
        int32_t tz = dists[vi];
        int32_t dx = (th->x - px) >> 12;
        int32_t dy = (th->y - py) >> 12;
        int32_t tx = (dx * sin_a - dy * cos_a) >> 14;

        int32_t sx = D_HALF_W + doom_sdiv(tx * D_PROJ, tz);
        int32_t spr_h_world = (th->kind >= TH_MEDIKIT && th->kind <= TH_ARMOR) ? 28
                            : (th->kind == TH_FIREBALL) ? 18
                            : (th->state == 3) ? 18 : 54;
        int32_t spr_w_world = (th->kind == TH_DEMON) ? 46
                            : (th->kind == TH_FIREBALL) ? 18
                            : (th->kind >= TH_MEDIKIT) ? 28 : 38;

        int32_t base_z = doom_st.secs[th->sector].floor_h + th->z;
        int32_t yb = D_HALF_H - doom_sdiv((base_z - view_z) * (D_PROJ << 4), tz);
        int32_t yt = D_HALF_H - doom_sdiv((base_z + spr_h_world - view_z) * (D_PROJ << 4), tz);
        int32_t sw = doom_sdiv(spr_w_world * (D_PROJ << 4), tz);
        if (sw < 2 || yb <= yt) continue;

        int32_t x0 = sx - (sw >> 1);
        int32_t x1 = x0 + sw - 1;
        if (x1 < 0 || x0 >= D_W) continue;

        int32_t sec_light = effective_light(&doom_st.secs[th->sector]);
        int32_t shade = (th->kind == TH_FIREBALL) ? 0 : (31 - (sec_light >> 3) + (tz >> 10));
        if (th->state == 2) shade = 0; /* bright pain flash */
        if (shade < 0) shade = 0;
        if (shade > 31) shade = 31;
        const uint8_t *cmap = doom_cmap[shade];
        const uint8_t *spr = doom_spr[th->kind - 1u];

        int32_t du = doom_sdiv(32 << 16, sw);
        int32_t dv = doom_sdiv(32 << 16, yb - yt);
        int32_t cx0 = x0 < 0 ? 0 : x0;
        int32_t cx1 = x1 >= D_W ? D_W - 1 : x1;
        int32_t cyt = yt < 0 ? 0 : yt;
        int32_t cyb = yb >= D_VIEWH ? D_VIEWH - 1 : yb;
        int32_t u = (cx0 - x0) * du;
        int32_t v0 = (cyt - yt) * dv;
        /* Flip sprite horizontally on odd walk frames for lively Doom walk cycle! */
        uint32_t flip = (th->kind <= TH_DEMON && th->state == 1 && ((doom_st.tic >> 2) & 1u)) ? 31u : 0u;

        for (int32_t x = cx0; x <= cx1; x++, u += du) {
            if (tz >= zbuf[x]) continue;
            uint32_t txu = ((uint32_t)u >> 16) ^ flip;
            if (txu >= 32u) continue;
            const uint8_t *scol = spr + (txu << 5);
            uint8_t *dst = doom_fb + cyt * D_W + x;
            int32_t v = v0;
            for (int32_t y = cyt; y <= cyb; y++, v += dv, dst += D_W) {
                uint8_t px_i = scol[((uint32_t)v >> 16) & 31u];
                if (px_i) *dst = cmap[px_i];
            }
        }
    }
}

/* Ultra-fast ARM 320x240 8bpp -> 640x480 RGBA4444 2x2 upscaler using 32-bit word loads & stmia bursts! */
__attribute__((noinline, target("arm")))
void doom_upscale_2x2(const uint8_t *src, uint16_t *dst, const uint32_t *pal32) {
    const uint32_t *sw = (const uint32_t *)(const void *)src;
    uint32_t *row0 = (uint32_t *)(void *)dst;
    uint32_t rows = D_H;
    uint32_t mask = 0x3FCu;
    __asm__ volatile(
        "1:\n\t"
        "add   r3, %1, #1280\n\t"           /* row1 = row0 + 640*2 bytes */
        "mov   r2, #80\n\t"                 /* 80 * 4 src px = 320 src px = 640 dst px */
        "2:\n\t"
        "ldr   r4, [%0], #4\n\t"            /* load 4 big-endian indexed pixels: p0 p1 p2 p3 */
        "and   r5, %4, r4, lsr #22\n\t"
        "ldr   r6, [%3, r5]\n\t"            /* p0 doubled RGBA4444 */
        "and   r5, %4, r4, lsr #14\n\t"
        "ldr   r7, [%3, r5]\n\t"            /* p1 doubled RGBA4444 */
        "and   r5, %4, r4, lsr #6\n\t"
        "ldr   r8, [%3, r5]\n\t"            /* p2 doubled RGBA4444 */
        "and   r5, %4, r4, lsl #2\n\t"
        "ldr   r9, [%3, r5]\n\t"            /* p3 doubled RGBA4444 */
        "stmia %1!, {r6-r9}\n\t"            /* 8 LCD pixels -> row 2y */
        "stmia r3!, {r6-r9}\n\t"            /* 8 LCD pixels -> row 2y+1 */
        "subs  r2, r2, #1\n\t"
        "bne   2b\n\t"
        "mov   %1, r3\n\t"                  /* advance row0 to row 2y+2 */
        "subs  %2, %2, #1\n\t"
        "bne   1b\n\t"
        : "+r"(sw), "+r"(row0), "+r"(rows)
        : "r"(pal32), "r"(mask)
        : "r2", "r3", "r4", "r5", "r6", "r7", "r8", "r9", "cc", "memory");
}
