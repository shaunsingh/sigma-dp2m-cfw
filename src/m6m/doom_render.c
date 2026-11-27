/* Bare-metal ARM926EJ-S Big-Endian DOOM 2.5D Sector/Portal Renderer & 2x2 stmia Upscaler */
#include "doom.h"

/* Portal clip arrays & visplane buffers in .bss */
static int16_t ytop_buf[D_W];
static int16_t ybot_buf[D_W];
static int32_t zbuf[D_W]; /* depth (Q4 map units) of nearest opaque column for sprite clipping */
static int16_t span_start[D_VIEWH];
static int16_t ceil_y0[D_W], ceil_y1[D_W];
static int16_t flr_y0[D_W], flr_y1[D_W];

struct portal_job {
    uint8_t sec, depth;
    int16_t x0, x1;
};

#define MAX_JOBS 48
static struct portal_job jobs[MAX_JOBS];
static int16_t job_ytop[MAX_JOBS][D_W];
static int16_t job_ybot[MAX_JOBS][D_W];

__attribute__((noinline, target("arm"), section(".doom.extra")))
uint32_t doom_udiv(uint32_t n, uint32_t d) {
    if (d <= 1u) return d ? n : 0x7FFFFFFFu;
    if (n < d) return 0u;
    uint32_t shift = (uint32_t)__builtin_clz(d) - (uint32_t)__builtin_clz(n);
    d <<= shift;
    uint32_t q = 0u;
    for (uint32_t i = shift + 1u; i; i--) {
        q <<= 1;
        if (n >= d) { n -= d; q |= 1u; }
        d >>= 1;
    }
    return q;
}

__attribute__((noinline, target("arm"), section(".doom.extra")))
int32_t doom_sdiv(int32_t n, int32_t d) {
    if (d == 0) return n >= 0 ? 0x3FFFFFFF : -0x3FFFFFFF;
    uint32_t neg = (uint32_t)(n ^ d) >> 31;
    uint32_t an = n < 0 ? 0u - (uint32_t)n : (uint32_t)n;
    uint32_t ad = d < 0 ? 0u - (uint32_t)d : (uint32_t)d;
    uint32_t q = doom_udiv(an, ad);
    return neg ? -(int32_t)q : (int32_t)q;
}

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

__attribute__((noinline, section(".doom.extra")))
static void draw_sky_col(int32_t x, int32_t y0, int32_t y1, uint32_t sky_x) {
    if (y0 > y1) return;
    int32_t mtn = 76 - ((d_sin(sky_x * 2u) >> 10) + (d_sin(sky_x * 5u + 170u) >> 11) + (d_sin(sky_x * 11u + 410u) >> 12));
    uint8_t *dst = doom_fb + y0 * D_W + x;
    for (int32_t y = y0; y <= y1; y++, dst += D_W) {
        if (y < mtn) {
            uint32_t g = (uint32_t)y >> 2;
            *dst = (uint8_t)(g < 15u ? 192u + g : 208u + ((g - 15u) & 7u));
        } else {
            int32_t dy = y - mtn;
            uint32_t ridge = ((sky_x ^ (uint32_t)dy) >> 1) & 3u;
            *dst = (uint8_t)(64u + ((dy > 14 ? 14 : dy) >> 1) + (ridge == 0));
        }
    }
}

__attribute__((always_inline, target("arm")))
static inline uint8_t flat_sample(const uint8_t *flat, const uint8_t *cmap, uint32_t u, uint32_t v) {
    return cmap[flat[((v >> 10) & 0xFC0u) | ((u >> 16) & 63u)]];
}

__attribute__((noinline, target("arm")))
static void draw_flat_span(int32_t y, int32_t x1, int32_t x2,
                           int32_t plane_dh, const uint8_t *flat,
                           int32_t sec_light, int32_t cos_a, int32_t sin_a,
                           int32_t px, int32_t py, uint32_t scroll) {
    int32_t dy = y - D_HALF_H;
    int32_t ady = dy ? d_abs(dy) : 1;
    int32_t adh = d_abs(plane_dh);
    if (adh < 1) adh = 1;

    uint32_t step_scale = doom_udiv((uint32_t)adh << 12, (uint32_t)ady);
    uint32_t dist_q12 = step_scale * (uint32_t)D_PROJ;
    const uint8_t *cmap = doom_cmap[d_shade(sec_light, (int32_t)(dist_q12 >> 18))];

    /* 64-bit smull prevents 32-bit overflow near horizon (ady = 1..3) */
    uint32_t d_q2 = dist_q12 >> 10;
    uint32_t fwd_x = d_q2 * (uint32_t)cos_a;
    uint32_t fwd_y = d_q2 * (uint32_t)sin_a;
    int32_t du = (int32_t)(((int64_t)(int32_t)step_scale * sin_a) >> 10);
    int32_t dv = (int32_t)(((int64_t)(int32_t)step_scale * -cos_a) >> 10);

    int32_t sx = x1 - D_HALF_W;
    uint32_t u = (uint32_t)px + fwd_x + (uint32_t)(sx * du) + scroll;
    uint32_t v = (uint32_t)py + fwd_y + (uint32_t)(sx * dv) + (scroll >> 1);

    uint8_t *dst = doom_fb + y * D_W + x1;
    int32_t cnt = x2 - x1 + 1;
    while (cnt > 0 && ((uintptr_t)dst & 3u)) {
        *dst++ = flat_sample(flat, cmap, u, v);
        u += (uint32_t)du; v += (uint32_t)dv; cnt--;
    }
    while (cnt >= 4) {
        uint32_t p0 = flat_sample(flat, cmap, u, v); u += (uint32_t)du; v += (uint32_t)dv;
        uint32_t p1 = flat_sample(flat, cmap, u, v); u += (uint32_t)du; v += (uint32_t)dv;
        uint32_t p2 = flat_sample(flat, cmap, u, v); u += (uint32_t)du; v += (uint32_t)dv;
        uint32_t p3 = flat_sample(flat, cmap, u, v); u += (uint32_t)du; v += (uint32_t)dv;
        *(uint32_t *)(void *)dst = (p0 << 24) | (p1 << 16) | (p2 << 8) | p3;
        dst += 4;
        cnt -= 4;
    }
    while (cnt-- > 0) {
        *dst++ = flat_sample(flat, cmap, u, v);
        u += (uint32_t)du; v += (uint32_t)dv;
    }
}

static void emit_visplane(int32_t x_lo, int32_t x_hi,
                          const int16_t *py0, const int16_t *py1,
                          int32_t plane_dh, uint8_t tex_id, int32_t sec_light,
                          int32_t cos_a, int32_t sin_a, int32_t px, int32_t py,
                          uint32_t scroll) {
    if (tex_id == TEX_SKY) {
        uint32_t base_sky = doom_st.angle << 1;
        for (int32_t x = x_lo; x <= x_hi; x++)
            if (py0[x] <= py1[x]) draw_sky_col(x, py0[x], py1[x], base_sky + (uint32_t)x);
        return;
    }
    const uint8_t *flat = doom_tex[tex_id < TEX_COUNT ? tex_id : FLAT_FLOOR1];
    int32_t prev_t = D_VIEWH, prev_b = -1;
    for (int32_t x = x_lo; x <= x_hi + 1; x++) {
        int32_t t = (x <= x_hi) ? py0[x] : D_VIEWH;
        int32_t b = (x <= x_hi) ? py1[x] : -1;
        if (t > b) { t = D_VIEWH; b = -1; }

        int32_t stop_t = t < prev_b + 1 ? t : prev_b + 1;
        for (int32_t y = prev_t; y < stop_t; y++)
            draw_flat_span(y, span_start[y], x - 1, plane_dh, flat, sec_light, cos_a, sin_a, px, py, scroll);
        int32_t stop_b = b > prev_t - 1 ? b : prev_t - 1;
        for (int32_t y = stop_b + 1; y <= prev_b; y++)
            draw_flat_span(y, span_start[y], x - 1, plane_dh, flat, sec_light, cos_a, sin_a, px, py, scroll);

        int32_t open_t = prev_t < b + 1 ? prev_t : b + 1;
        for (int32_t y = t; y < open_t; y++) span_start[y] = (int16_t)x;
        int32_t open_b = prev_b > t - 1 ? prev_b : t - 1;
        for (int32_t y = open_b + 1; y <= b; y++) span_start[y] = (int16_t)x;

        prev_t = t;
        prev_b = b;
    }
}

static int32_t effective_light(const struct d_sector_def *sd) {
    int32_t l = sd->light;
    if ((sd->flags & SEC_BLINK) && ((doom_st.tic >> 3) & 3u) == 0u)
        l = l > 80 ? l - 80 : 32;
    l += (int32_t)doom_st.flash_tics * 24;
    return l > 255 ? 255 : l;
}

static void draw_sprites(uint32_t vis_sec_mask, int32_t px, int32_t py,
                         int32_t view_z, int32_t cos_a, int32_t sin_a) {
    int32_t order[MAX_THINGS], dists[MAX_THINGS], n_vis = 0;
    for (int32_t i = 0; i < MAX_THINGS; i++) {
        const struct d_thing *th = &doom_st.things[i];
        if (th->kind == TH_NONE || !(vis_sec_mask & (1u << th->sector))) continue;
        int32_t dx = (th->x - px) >> 12, dy = (th->y - py) >> 12;
        int32_t tz = (dx * cos_a + dy * sin_a) >> 14;
        if (tz < 64) continue;
        order[n_vis] = i;
        dists[n_vis++] = tz;
    }

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
        int32_t dx = (th->x - px) >> 12, dy = (th->y - py) >> 12;
        int32_t tx = (dx * sin_a - dy * cos_a) >> 14;
        int32_t sx = D_HALF_W + doom_sdiv(tx * D_PROJ, tz);

        int32_t spr_h = (th->kind >= TH_MEDIKIT && th->kind <= TH_ARMOR) ? 28
                      : (th->kind == TH_FIREBALL || th->state == 3) ? 18 : 54;
        int32_t spr_w = (th->kind == TH_DEMON) ? 46
                      : (th->kind == TH_FIREBALL) ? 18
                      : (th->kind >= TH_MEDIKIT) ? 28 : 38;

        int32_t base_z = doom_map_secs[th->sector].floor_h + th->z;
        int32_t yb = D_HALF_H - doom_sdiv((base_z - view_z) * (D_PROJ << 4), tz);
        int32_t yt = D_HALF_H - doom_sdiv((base_z + spr_h - view_z) * (D_PROJ << 4), tz);
        int32_t sw = doom_sdiv(spr_w * (D_PROJ << 4), tz);
        if (sw < 2 || yb <= yt) continue;

        int32_t x0 = sx - (sw >> 1), x1 = x0 + sw - 1;
        if (x1 < 0 || x0 >= D_W) continue;

        int32_t shade = (th->kind == TH_FIREBALL || th->state == 2) ? 0
                      : d_shade(effective_light(&doom_map_secs[th->sector]), tz >> 10);
        const uint8_t *cmap = doom_cmap[shade];
        const uint8_t *spr = doom_spr[th->kind - 1u];

        int32_t du = doom_sdiv(32 << 16, sw), dv = doom_sdiv(32 << 16, yb - yt);
        int32_t cx0 = x0 < 0 ? 0 : x0, cx1 = x1 >= D_W ? D_W - 1 : x1;
        int32_t cyt = yt < 0 ? 0 : yt, cyb = yb >= D_VIEWH ? D_VIEWH - 1 : yb;
        int32_t u = (cx0 - x0) * du, v0 = (cyt - yt) * dv;
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

void doom_render_3d(void) {
    int32_t px = doom_st.px, py = doom_st.py, view_z = doom_st.view_z;
    int32_t cos_a = d_cos(doom_st.angle), sin_a = d_sin(doom_st.angle);
    uint32_t nuk_scroll = doom_st.tic << 15;

    for (int32_t x = 0; x < D_W; x++) {
        zbuf[x] = 0x7FFFFFFF;
        job_ytop[0][x] = 0;
        job_ybot[0][x] = D_VIEWH - 1;
    }

    int32_t q_head = 0, q_tail = 1;
    jobs[0] = (struct portal_job){ doom_st.sector, 0, 0, D_W - 1 };
    uint32_t vis_sec_mask = 0u;

    while (q_head < q_tail) {
        int32_t ji = q_head++;
        uint8_t sec_id = jobs[ji].sec, depth = jobs[ji].depth;
        int32_t win_x0 = jobs[ji].x0, win_x1 = jobs[ji].x1;
        const struct d_sector_def *sd = &doom_map_secs[sec_id];
        int32_t sec_ceil = doom_st.secs[sec_id].ceil_h;
        int32_t sec_light = effective_light(sd);
        vis_sec_mask |= 1u << sec_id;

        for (int32_t x = win_x0; x <= win_x1; x++) {
            ytop_buf[x] = job_ytop[ji][x];
            ybot_buf[x] = job_ybot[ji][x];
            ceil_y0[x] = flr_y0[x] = D_VIEWH;
            ceil_y1[x] = flr_y1[x] = -1;
        }

        int32_t c_dh = sec_ceil - view_z, f_dh = sd->floor_h - view_z;
        int32_t tex_h = sec_ceil - sd->floor_h;
        const struct d_line *lines = &doom_map_lines[sd->first_line];

        for (int32_t li = 0; li < sd->num_lines; li++) {
            const struct d_line *ln = &lines[li];
            int32_t dx1 = ((int32_t)ln->x1 << 4) - (px >> 12);
            int32_t dy1 = ((int32_t)ln->y1 << 4) - (py >> 12);
            int32_t dx2 = ((int32_t)ln->x2 << 4) - (px >> 12);
            int32_t dy2 = ((int32_t)ln->y2 << 4) - (py >> 12);

            int32_t tz1 = (dx1 * cos_a + dy1 * sin_a) >> 14;
            int32_t tx1 = (dx1 * sin_a - dy1 * cos_a) >> 14;
            int32_t tz2 = (dx2 * cos_a + dy2 * sin_a) >> 14;
            int32_t tx2 = (dx2 * sin_a - dy2 * cos_a) >> 14;

            const int32_t NEAR_Z = 32; /* 2.0 map units in Q4 */
            if (tz1 < NEAR_Z && tz2 < NEAR_Z) continue;

            int32_t wdx = (int32_t)ln->x2 - ln->x1, wdy = (int32_t)ln->y2 - ln->y1;
            int32_t u1 = 0, u2 = d_oct_dist(wdx, wdy) << 4;

            if (tz1 < NEAR_Z) {
                int32_t num = NEAR_Z - tz1, den = tz2 - tz1;
                tx1 += doom_sdiv(num * (tx2 - tx1), den);
                u1  += doom_sdiv(num * (u2 - u1), den);
                tz1 = NEAR_Z;
            } else if (tz2 < NEAR_Z) {
                int32_t num = NEAR_Z - tz2, den = tz1 - tz2;
                tx2 += doom_sdiv(num * (tx1 - tx2), den);
                u2  += doom_sdiv(num * (u1 - u2), den);
                tz2 = NEAR_Z;
            }

            /* CCW walls viewed from inside project right-to-left (sx2 < sx1) */
            int32_t xR = D_HALF_W + doom_sdiv(tx1 * D_PROJ, tz1);
            int32_t xL = D_HALF_W + doom_sdiv(tx2 * D_PROJ, tz2);
            int32_t span_x = xR - xL;
            if (span_x <= 0 || xR < win_x0 || xL > win_x1) continue;

            int32_t zL = tz2, zR = tz1, uL = u2, uR = u1;
            int32_t cx0 = xL < win_x0 ? win_x0 : xL;
            int32_t cx1 = xR > win_x1 ? win_x1 : xR;

            int32_t y1a = D_HALF_H - doom_sdiv(c_dh * (D_PROJ << 4), zL);
            int32_t y1b = D_HALF_H - doom_sdiv(f_dh * (D_PROJ << 4), zL);
            int32_t y2a = D_HALF_H - doom_sdiv(c_dh * (D_PROJ << 4), zR);
            int32_t y2b = D_HALF_H - doom_sdiv(f_dh * (D_PROJ << 4), zR);

            int32_t izL = (int32_t)doom_udiv(1u << 24, (uint32_t)zL);
            int32_t izR = (int32_t)doom_udiv(1u << 24, (uint32_t)zR);
            int32_t uzL = doom_sdiv(uL << 12, zL);
            int32_t uzR = doom_sdiv(uR << 12, zR);

            int32_t back_id = ln->back;
            int32_t by1a = 0, by1b = 0, by2a = 0, by2b = 0;
            if (back_id >= 0) {
                int32_t bc_dh = doom_st.secs[back_id].ceil_h - view_z;
                int32_t bf_dh = doom_map_secs[back_id].floor_h - view_z;
                by1a = D_HALF_H - doom_sdiv(bc_dh * (D_PROJ << 4), zL);
                by1b = D_HALF_H - doom_sdiv(bf_dh * (D_PROJ << 4), zL);
                by2a = D_HALF_H - doom_sdiv(bc_dh * (D_PROJ << 4), zR);
                by2b = D_HALF_H - doom_sdiv(bf_dh * (D_PROJ << 4), zR);
            }

            int32_t wall_light = d_clamp(sec_light + (d_abs(wdx) > d_abs(wdy) ? 16 : -16), 0, 255);
            const uint8_t *tex = doom_tex[ln->tex < TEX_COUNT ? ln->tex : TEX_STARTAN];
            const uint8_t *low_tex = (back_id >= 0 && doom_map_secs[back_id].floor_h > sd->floor_h)
                                         ? doom_tex[TEX_STEP] : tex;

            int32_t p_x0 = D_W, p_x1 = -1;
            int32_t child_ji = (back_id >= 0 && depth < 10 && q_tail < MAX_JOBS) ? q_tail : -1;

            int32_t dy_top = doom_sdiv((int32_t)((uint32_t)(y2a - y1a) << 16), span_x);
            int32_t dy_bot = doom_sdiv((int32_t)((uint32_t)(y2b - y1b) << 16), span_x);
            int32_t diz    = doom_sdiv(izR - izL, span_x);
            int32_t duz    = doom_sdiv(uzR - uzL, span_x);
            int32_t dby_t  = back_id >= 0 ? doom_sdiv((int32_t)((uint32_t)(by2a - by1a) << 16), span_x) : 0;
            int32_t dby_b  = back_id >= 0 ? doom_sdiv((int32_t)((uint32_t)(by2b - by1b) << 16), span_x) : 0;

            int32_t off = cx0 - xL;
            int32_t cur_yt = (int32_t)((uint32_t)y1a << 16) + off * dy_top;
            int32_t cur_yb = (int32_t)((uint32_t)y1b << 16) + off * dy_bot;
            int32_t cur_iz = izL + off * diz;
            int32_t cur_uz = uzL + off * duz;
            int32_t cur_bt = (int32_t)((uint32_t)by1a << 16) + off * dby_t;
            int32_t cur_bb = (int32_t)((uint32_t)by1b << 16) + off * dby_b;

            for (int32_t x = cx0; x <= cx1; x++) {
                int32_t top = ytop_buf[x], bot = ybot_buf[x];
                if (top <= bot) {
                    int32_t ya = cur_yt >> 16, yb = cur_yb >> 16;
                    int32_t cya = d_clamp(ya, top, bot + 1);
                    int32_t cyb = d_clamp(yb, top - 1, bot);

                    if (cya > top) { ceil_y0[x] = (int16_t)top;     ceil_y1[x] = (int16_t)(cya - 1); }
                    if (cyb < bot) { flr_y0[x]  = (int16_t)(cyb + 1); flr_y1[x]  = (int16_t)bot; }

                    int32_t iz = cur_iz > 256 ? cur_iz : 256;
                    int32_t z_q4 = (int32_t)doom_udiv(1u << 24, (uint32_t)iz);
                    uint32_t tex_u = ((uint32_t)doom_sdiv(cur_uz, iz >> 8)) & 63u;
                    const uint8_t *cmap = doom_cmap[d_shade(wall_light, z_q4 >> 10)];

                    int32_t wall_h_px = yb - ya;
                    if (wall_h_px < 1) wall_h_px = 1;

                    if (back_id < 0) {
                        int32_t dv = doom_sdiv(tex_h << 16, wall_h_px);
                        draw_wall_col(x, cya, cyb, tex + (tex_u << 6), cmap, (cya - ya) * dv, dv);
                        if (z_q4 < zbuf[x]) zbuf[x] = z_q4;
                    } else {
                        int32_t bya = cur_bt >> 16, byb = cur_bb >> 16;
                        int32_t cbya = d_clamp(bya, cya, bot + 1);
                        int32_t cbyb = d_clamp(byb, top - 1, cyb);

                        if (cbya > cya || cyb > cbyb) {
                            int32_t dv = doom_sdiv(tex_h << 16, wall_h_px);
                            if (cbya > cya) {
                                int32_t v = (ln->flags & LINE_DOOR)
                                          ? ((cya - bya) * dv) + (64 << 16)
                                          : (cya - ya) * dv;
                                draw_wall_col(x, cya, cbya - 1, tex + (tex_u << 6), cmap, v, dv);
                            }
                            if (cyb > cbyb)
                                draw_wall_col(x, cbyb + 1, cyb, low_tex + (tex_u << 6), cmap, (cbyb + 1 - byb) * dv, dv);
                        }

                        int32_t ntop = cbya > cya ? cbya : cya;
                        int32_t nbot = cbyb < cyb ? cbyb : cyb;
                        if (ntop <= nbot && child_ji >= 0) {
                            if (x < p_x0) p_x0 = x;
                            if (x > p_x1) p_x1 = x;
                            job_ytop[child_ji][x] = (int16_t)ntop;
                            job_ybot[child_ji][x] = (int16_t)nbot;
                        } else {
                            if (child_ji >= 0) { job_ytop[child_ji][x] = D_VIEWH; job_ybot[child_ji][x] = -1; }
                            if (z_q4 < zbuf[x]) zbuf[x] = z_q4;
                        }
                    }
                    ytop_buf[x] = (int16_t)(bot + 1);
                } else if (child_ji >= 0) {
                    job_ytop[child_ji][x] = D_VIEWH;
                    job_ybot[child_ji][x] = -1;
                }
                cur_yt += dy_top; cur_yb += dy_bot;
                cur_iz += diz;    cur_uz += duz;
                cur_bt += dby_t;  cur_bb += dby_b;
            }

            if (child_ji >= 0 && p_x0 <= p_x1) {
                jobs[child_ji] = (struct portal_job){ (uint8_t)back_id, (uint8_t)(depth + 1u), (int16_t)p_x0, (int16_t)p_x1 };
                q_tail++;
            }
        }

        uint32_t flr_scroll = (sd->flags & SEC_NUKAGE) ? nuk_scroll : 0u;
        emit_visplane(win_x0, win_x1, ceil_y0, ceil_y1, c_dh, sd->ceil_tex, sec_light, cos_a, sin_a, px, py, 0u);
        emit_visplane(win_x0, win_x1, flr_y0, flr_y1, f_dh, sd->floor_tex, sec_light, cos_a, sin_a, px, py, flr_scroll);
    }

    draw_sprites(vis_sec_mask, px, py, view_z, cos_a, sin_a);
}

/* ARM 320x240 8bpp -> 640x480 RGBA4444 2x2 upscaler using word loads & stmia bursts */
__attribute__((noinline, target("arm")))
void doom_upscale_2x2(const uint8_t *src, uint16_t *dst, const uint32_t *pal32) {
    const uint32_t *sw = (const uint32_t *)(const void *)src;
    uint32_t *row0 = (uint32_t *)(void *)dst;
    uint32_t rows = D_H;
    uint32_t mask = 0x3FCu;
    __asm__ volatile(
        "1:\n\t"
        "add   r3, %1, #1280\n\t"
        "mov   r2, #80\n\t"
        "2:\n\t"
        "ldr   r4, [%0], #4\n\t"
        "and   r5, %4, r4, lsr #22\n\t"
        "ldr   r6, [%3, r5]\n\t"
        "and   r5, %4, r4, lsr #14\n\t"
        "ldr   r7, [%3, r5]\n\t"
        "and   r5, %4, r4, lsr #6\n\t"
        "ldr   r8, [%3, r5]\n\t"
        "and   r5, %4, r4, lsl #2\n\t"
        "ldr   r9, [%3, r5]\n\t"
        "stmia %1!, {r6-r9}\n\t"
        "stmia r3!, {r6-r9}\n\t"
        "subs  r2, r2, #1\n\t"
        "bne   2b\n\t"
        "mov   %1, r3\n\t"
        "subs  %2, %2, #1\n\t"
        "bne   1b\n\t"
        : "+r"(sw), "+r"(row0), "+r"(rows)
        : "r"(pal32), "r"(mask)
        : "r2", "r3", "r4", "r5", "r6", "r7", "r8", "r9", "cc", "memory");
}
