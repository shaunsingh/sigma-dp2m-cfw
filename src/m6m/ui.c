// settings, menus, strings, capture IPC packing & the Film menu page
#include "dp2m.h"
#include "catalog.h"

static uint32_t film_paper(const struct film_info *f) {
    static const uint32_t slot[] = { 0, NV_COLOR_PAPER, NV_BW_PAPER, NV_LUT_SLOT };
    if (!f->paper_cls) return 0;
    uint32_t p = REG8(slot[f->paper_cls]);
    return (p - 1u < 8u) ? p - 1u : f->auto_paper;
}

static const struct film_info *film_of(uint32_t cm) {
    return (cm - FILM_MODE0 < dp2m_film_count) ? &dp2m_films[cm - FILM_MODE0] : 0;
}

uint32_t packed_cm(uint32_t cm) {
    const struct film_info *f = film_of(cm);
    return f ? f->packed + film_paper(f) : cm;
}

// cmd[5] = packed mode, cmd[6] = (fill << 5) | (fx << 2) | cs with fx = 3d | grain << 1 | halation << 2
void ipc_pack(uint8_t *cmd, const uint8_t *meta) {
    uint32_t fx = (REG8(NV_ENGINE) & 1u) | (REG8(NV_GRAIN) & 1u) << 1 | (REG8(NV_HALATION) & 1u) << 2;
    cmd[5] = (uint8_t)packed_cm(meta[0x1E]);
    cmd[6] = (uint8_t)((meta[0x1D] & 3u) | fx << 2 | (REG8(NV_FILL) & 3u) << 5);
}

#define NV_SETTING(name, addr, max, after)                                             \
    uint32_t nv_get_##name(void) { uint32_t v = REG8(addr); return v <= (max) ? v : 0; } \
    void nv_set_##name(uint32_t v) { REG8(addr) = (uint8_t)v; after; }

NV_SETTING(color_paper, NV_COLOR_PAPER, 8u, stock_lv_refresh())
NV_SETTING(bw_paper, NV_BW_PAPER, 8u, stock_lv_refresh())
NV_SETTING(lut_slot, NV_LUT_SLOT, 8u, (void)0)
NV_SETTING(engine, NV_ENGINE, 1u, (void)0)
NV_SETTING(grain, NV_GRAIN, 1u, (void)0)
NV_SETTING(halation, NV_HALATION, 1u, (void)0)
NV_SETTING(fill, NV_FILL, 3u, (void)0)
NV_SETTING(minshutter, NV_MINSHUTTER, 6u, (void)0)
NV_SETTING(peakcol, NV_PEAKCOL, 3u, (void)0)

uint32_t nv_get_none(void) { return 0; }
void nv_set_none(uint32_t v) { (void)v; }

static uint32_t paper_cls(void) {
    const struct film_info *f = film_of(stock_get_cm());
    return f ? f->paper_cls : 0;
}

const struct menu_row *menu_lookup(uint32_t id) {
    if ((id & ~MENU_QS_COPY) == MENU_PAPER) id += MENU_PAPER_CLS0 - MENU_PAPER + paper_cls();
    for (const struct menu_map *m = dp2m_menus; m->rows; m++)
        if (m->id == id) return m->rows;
    return 0;
}

const char *dp2m_str(uint32_t sid, uint32_t lang) {
    (void)lang;
    if (sid == SID_PAPER_TITLE) sid += 1u + paper_cls();
    uint32_t i = sid - DP2M_SID0;
    return i < dp2m_string_count ? dp2m_strings[i] : 0;
}

// LV info icon: films show their own label (osd_map)
uint32_t osd_cm(void) {
    return osd_map(stock_get_cm());
}
HOOK_CALL(0xA016BC4E, osd_cm, 0xA0167926);

// Film page inserted as menu page 2: shoot 0..4, playback 5..6, setup 7..10
uint32_t page_group(uint32_t page) {
    return page < 5u ? 0u : page < 7u ? 1u : 2u;
}
HOOK_CALL(0xA005811C, page_group, 0xA0058994);
HOOK_CALL(0xA0058144, page_group, 0xA0058994);
HOOK_CALL(0xA0058204, page_group, 0xA0058994);
HOOK_CALL(0xA00582C6, page_group, 0xA0058994);
HOOK_CALL(0xA0058308, page_group, 0xA0058994);
HOOK_CALL(0xA00584E8, page_group, 0xA0058994);

// page lists 0x0E 0x0F FILM 0x10..0x17: menu entry (0xA0058A0A) copies the template at state + 0x14 over
// the live list at + 0x3C, so both change and the copy grows to 11 pages (template slot 10 held factory page 0x18)
#define PAGE(i, id)                                \
    HOOK_HALF(0xA07F22E6 + 2 * (i), id, 0x0E + (i)); \
    HOOK_HALF(0xA07F230E + 2 * (i), id, (i) < 10 ? 0x0E + (i) : 0)
PAGE(2, MENU_FILM_PAGE);
PAGE(3, 0x10);
PAGE(4, 0x11);
PAGE(5, 0x12);
PAGE(6, 0x13);
PAGE(7, 0x14);
PAGE(8, 0x15);
PAGE(9, 0x16);
PAGE(10, 0x17);
HOOK_HALF(0xA0058A12, 0x2216, 0x2214);

// page stepping wraps at index 10, tab strip draws 11 tabs 52 px apart
HOOK_HALF(0xA005868A, 0x210A, 0x2109);
HOOK_HALF(0xA00586A0, 0x210A, 0x2109);
HOOK_HALF(0xA005871C, 0x210A, 0x2109);
HOOK_HALF(0xA0058734, 0x210A, 0x2109);
HOOK_HALF(0xA0058454, 0x2D0B, 0x2D0A);
HOOK_HALF(0xA005844A, 0x3034, 0x3038);

// playback mode (expo 8) pins the menu to the playback pages, now 5..6
HOOK_HALF(0xA0058806, 0x2805, 0x2804);
HOOK_HALF(0xA005880A, 0x2806, 0x2805);
HOOK_HALF(0xA005880E, 0x2005, 0x2004);
HOOK_HALF(0xA005881E, 0x2805, 0x2804);
HOOK_HALF(0xA0058822, 0x2806, 0x2805);

// tab geometry 0xA04F3100: first tab {x,y,w,h}, group headers & group icons for 5/2/4 pages
HOOK_HALF(0xA04F3100, 34, 0x28);
HOOK_HALF(0xA04F3104, 48, 0x34);
HOOK_HALF(0xA04F3110, 32, 0x26);
HOOK_HALF(0xA04F3114, 260, 0xE0);
HOOK_HALF(0xA04F3118, 292, 0x106);
HOOK_HALF(0xA04F311C, 104, 0x70);
HOOK_HALF(0xA04F3120, 396, 0x176);
HOOK_HALF(0xA04F3124, 208, 0xE0);
HOOK_HALF(0xA04F3128, 143, 0x84);
HOOK_HALF(0xA04F3130, 325, 0x12A);
HOOK_HALF(0xA04F3138, 481, 0x1D0);
