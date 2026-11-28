/* SPDX-License-Identifier: GPL-2.0-or-later */
/* Hybrid DP2M WAD Backend: SDRAM Preload (<=6 MB w/ Boot Progress) + MDF Sector Cache */
#include <stdio.h>
#include <stdlib.h>
#include <string.h>

#include "config.h"
#include "doomtype.h"
#include "w_file.h"
#include "z_zone.h"
#include "dp2m_doom.h"

#define WAD_PRELOAD_MAX (6u * 1024u * 1024u)
#define WAD_CACHE_SIZE  16384u

typedef struct {
    wad_file_t wad;
    int32_t    fd;
    uint8_t   *ram_data;
    uint8_t   *cache_buf;
    uint32_t   cache_base;
    uint32_t   cache_len;
} dp2m_wad_file_t;

extern wad_file_class_t stdc_wad_file;

static uint32_t probe_wad_len(int32_t fd) {
    /* Stock mdf_fstat (0xA0111563) writes up to +0x224; allocate 0x240 bytes */
    uint8_t st[0x240] __attribute__((aligned(8)));
    memset(st, 0, sizeof(st));
    if (g_dp2m_api->mdf_fstat && g_dp2m_api->mdf_fstat(fd, st) == 0) {
        uint32_t sz = *(const uint32_t *)(const void *)(st + 0x214);
        if (sz > 0u) return sz;
    }
    uint32_t end_pos = 0u, zero_pos = 0u;
    if (g_dp2m_api->mdf_lseek(fd, 0, 2u, &end_pos) == 0) {
        g_dp2m_api->mdf_lseek(fd, 0, 0u, &zero_pos);
        return end_pos;
    }
    return 0u;
}

static wad_file_t *W_DP2M_OpenFile(char *path) {
    int32_t fd = dp2m_sd_open_read(path);
    if (fd < 0) return NULL;

    uint32_t len = probe_wad_len(fd);
    if (len < 12u) {
        dp2m_sd_close(fd);
        return NULL;
    }

    dp2m_wad_file_t *dw = (dp2m_wad_file_t *)Z_Malloc(sizeof(*dw), PU_STATIC, 0);
    if (!dw) {
        dp2m_sd_close(fd);
        return NULL;
    }
    memset(dw, 0, sizeof(*dw));
    dw->wad.file_class = &stdc_wad_file;
    /* Keep wad.mapped = NULL so W_CacheLumpNum copies lumps into Zone blocks.
     * This prevents ARMv5 unaligned lump faults AND P_LoadBlockMap BE double-swap! */
    dw->wad.mapped = NULL;
    dw->wad.length = len;
    dw->fd = fd;
    dw->cache_base = 0xFFFFFFFFu;

    /* Record WAD basename + size for the Tech Demo Dashboard */
    const char *base = path;
    for (const char *p = path; *p; p++) {
        if (*p == '/' || *p == '\\' || *p == ':') base = p + 1;
    }
    size_t ni = 0;
    for (; base[ni] && ni + 1 < sizeof(g_doom_tel.wad_name); ni++)
        g_doom_tel.wad_name[ni] = (char)((base[ni] >= 'a' && base[ni] <= 'z') ? (base[ni] - 32) : base[ni]);
    g_doom_tel.wad_name[ni] = '\0';
    g_doom_tel.wad_size_bytes = len;

    /* Try preloading WAD into 17 MB stock SDRAM heap for zero-stall 35 Hz gameplay */
    if (len <= WAD_PRELOAD_MAX) {
        uint8_t *buf = (uint8_t *)malloc(len);
        if (buf) {
            uint32_t zero_pos = 0u;
            g_dp2m_api->mdf_lseek(fd, 0, 0u, &zero_pos);
            uint32_t pos = 0u;
            int ok = 1;
            while (pos < len) {
                uint32_t chunk = (len - pos > 131072u) ? 131072u : (len - pos);
                uint32_t got = 0u;
                if (g_dp2m_api->mdf_read(fd, buf + pos, chunk, &got) != 0 || got == 0u || got > chunk) {
                    ok = 0;
                    break;
                }
                pos += got;
                dp2m_draw_boot_progress("PRELOADING WAD TO SDRAM", pos, len);
            }
            if (ok) {
                dp2m_sd_close(fd);
                dw->fd = -1;
                dw->ram_data = buf;
                g_doom_tel.wad_preloaded = 1u;
                return &dw->wad;
            }
            free(buf);
            g_dp2m_api->mdf_lseek(fd, 0, 0u, &zero_pos);
        }
    }

    /* Fallback for large IWADs (DOOM2.WAD 14.6 MB): 16 KB sector read-ahead cache */
    dw->cache_buf = (uint8_t *)malloc(WAD_CACHE_SIZE);
    g_doom_tel.wad_preloaded = 0u;
    return &dw->wad;
}

static void W_DP2M_CloseFile(wad_file_t *wad) {
    dp2m_wad_file_t *dw = (dp2m_wad_file_t *)wad;
    if (!dw) return;
    if (dw->fd >= 0) {
        dp2m_sd_close(dw->fd);
        dw->fd = -1;
    }
    if (dw->ram_data) {
        free(dw->ram_data);
        dw->ram_data = NULL;
    }
    if (dw->cache_buf) {
        free(dw->cache_buf);
        dw->cache_buf = NULL;
    }
    Z_Free(dw);
}

static size_t W_DP2M_Read(wad_file_t *wad, unsigned int offset,
                          void *buffer, size_t buffer_len) {
    dp2m_wad_file_t *dw = (dp2m_wad_file_t *)wad;
    if (!dw || offset >= dw->wad.length || buffer_len == 0) return 0;
    if (offset + buffer_len > dw->wad.length)
        buffer_len = dw->wad.length - offset;

    g_doom_tel.wad_read_calls++;
    g_doom_tel.wad_read_bytes += (uint32_t)buffer_len;
    g_doom_tel.wad_last_off = offset;

    if (dw->ram_data) {
        memcpy(buffer, dw->ram_data + offset, buffer_len);
        return buffer_len;
    }

    if (dw->fd < 0) return 0;

    /* Serve small lump/directory reads out of 16 KB read-ahead cache */
    if (dw->cache_buf && buffer_len <= (WAD_CACHE_SIZE >> 1)) {
        if (offset < dw->cache_base || offset + buffer_len > dw->cache_base + dw->cache_len) {
            uint32_t aligned = offset & ~511u;
            uint32_t pos = 0u, got = 0u;
            if (g_dp2m_api->mdf_lseek(dw->fd, (int32_t)aligned, 0u, &pos) == 0 &&
                g_dp2m_api->mdf_read(dw->fd, dw->cache_buf, WAD_CACHE_SIZE, &got) == 0 &&
                got <= WAD_CACHE_SIZE && got > (offset - aligned)) {
                dw->cache_base = aligned;
                dw->cache_len = got;
            } else {
                dw->cache_len = 0u;
            }
        }
        if (offset >= dw->cache_base && offset + buffer_len <= dw->cache_base + dw->cache_len) {
            memcpy(buffer, dw->cache_buf + (offset - dw->cache_base), buffer_len);
            return buffer_len;
        }
    }

    uint32_t pos = 0u;
    if (g_dp2m_api->mdf_lseek(dw->fd, (int32_t)offset, 0u, &pos) != 0)
        return 0;

    uint8_t *dst = (uint8_t *)buffer;
    size_t total = 0;
    while (total < buffer_len) {
        uint32_t chunk = (buffer_len - total > 65536u) ? 65536u : (uint32_t)(buffer_len - total);
        uint32_t got = 0u;
        if (g_dp2m_api->mdf_read(dw->fd, dst + total, chunk, &got) != 0 || got == 0u || got > chunk)
            break;
        total += got;
    }
    return total;
}

wad_file_class_t stdc_wad_file = {
    W_DP2M_OpenFile,
    W_DP2M_CloseFile,
    W_DP2M_Read,
};
