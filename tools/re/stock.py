"""Named stock S3 addresses of the DP2M v1.05 M6M (RE findings shared by the tools and the verifier)."""

# UI
GET_MENU_TABLE = 0xA004CD08  # (id) -> 32 B rows, zero row terminated
UI_STRING = 0xA0044AAC  # (sid, lang) -> char *
STR_TBL, STR_STRIDE, STR_LANGS = 0xA07DF144, 0x44, 17  # per sid: one pointer per language
STOCK_SIDS = 672
SCREEN_VT = 0xA07DEB28  # handler of posted screen id = *(SCREEN_VT + 4 * id)
SCREEN_POST = 0xA0042AEC  # (type, id)
ICON_GET = 0xA0042AAC  # (id) -> {u32 bits, u16 w, u16 h} or 0
ICON_RESULT = 0xA07DD77C  # the struct ICON_GET returns
ICON_TBL, ICON_MAX = 0xA07DD784, 0x1A1  # 12 B per id: ARM getters of bits, w, h
CLUT, CLUT_N = 0xA07DEC54, 0xDB  # [A, Y, Cb, Cr], loaded by 0xA00434EC
ICON_HI = 0xC8  # enabled rows blit mode 1: index = hi + 1 - v (v 1 = white fill, 16 = black outline)
OSD = 0xA2130000  # 640x480 8bpp index plane
BLIT = 0xA0040DD8  # (src, x, y, w, [sp] h, hi, lo, transparent, mode)
FILL_BOX = 0xA0041198
QS_STATE = 0xA07DF008  # +4 page, +6 strip scroll, +8 + 2 * page focused slot
QS_SLOT_ROWS = 0xA00435D8  # (page, slot) -> rows
QS_STRIP = 0xA0043E5C
QS_STRIP_CHECK = 0xA0043E9C  # stock routes only the ISO getter to the scrolling strip
MENU_ROW_COUNT = 0xA004D4D6
ROW_OSD_ICON = 0xA0044FCC  # (rows, ?) -> w2 high of the current value's row
LV_STATUS_CM = 0xA004C7BC
LV_INFO_CM = 0xA016BC4A
STRIP_BG_CLEAR = 0xA0044E48  # waits on the 2D engine

# font engine (only engine 0 is live; BP4 outline fonts)
TEXT_DRAW = 0xA0042684  # (lang, str, maxlen, px, [sp] 5, 0, 1, rect *, style, 0); rect y >= 480 is offscreen
LINE_BUF, LINE_W, LINE_ROWS = 0xA093A514, 640, 256  # coverage 1..15, 1 = solid
FONT_SCALE = 0xA07DD774  # (num, den), set per screen
GET_LANG = 0xA0167DD4
UITRON = (0xA0070000, 0xA0076000)  # kernel service calls

# settings & NVRAM
NV_BLOCK, NV_LEN, NV_PERSIST = 0xA0939684, 0x44C, 0x3C4
NV_DEFAULTS = 0xA07F9E68
NV_CM = 0xA09396C3
NV_BKT_ORD = 0xA09396DF
NV_EXPO = 0xA09396AC
NV_CUR = 0xA09396C0  # shooting settings (0x8C B) a custom slot stores
NV_LOAD = 0xA0040110  # (dst) boot load of NOR bank 0
NV_FLUSH = 0xA004017A  # NVRAM -> NOR bank 0 record at 8 (stock block at 0x20)
NV_RESET = 0xA0167574
NOR_READ, NOR_WRITE = 0xA00206CC, 0xA00206FA  # (off, n, buf), bank 0
MALLOC, FREE = 0xA003D9C0, 0xA003D9AC
GET_CM = 0xA0167926
GET_EXPO_MODE = 0xA0167892
BKT_COUNT = 0xA0168100
BKT_EV_Q16 = 0xA0002FE4  # (shot) -> EV offset q16
BKT_STEP_IDX = 0xA07FBE5A
AUTOISO_A = 0xA000D83E  # A mode min shutter `ldr r0,=..; b +8`
AUTOISO_P = (0xA000D9A2, 0xA000D9E8)  # P mode `movs r0,#9; lsls r0,#16`
IPC_CMD20 = 0xA00360A2  # capture IPC cmd 0x20 fill: r4 cmd, meta at sp + 0x14
LV_YCAP = 0xA0033F1A  # LV YCC capture, r0 -> {y, cb, cr}
LV_REFRESH_DIRTY = 0xA003A260
LV_GAMMA_SITE = 0xA0039890
LV_MONO_SITE = 0xA003A1E6
LV_CMCM_SITE = 0xA003A20A

# updater screen
VERSION_IMM = 0xA005F87C  # "Current version" sprintf("%d.%02d", movs r2,#maj; movs r3,#min)
UPDATER_BANNERS = (0xA005FC3C, 0xA005FC5C, 0xA005FC7C)  # debug-write banners for types 2/3/4

NAMES = {
    GET_MENU_TABLE: "get_menu_table",
    UI_STRING: "ui_string",
    SCREEN_POST: "screen_post",
    ICON_GET: "icon_get",
    BLIT: "osd_blit",
    TEXT_DRAW: "text_draw",
    0xA0042274: "text_measure",
    0xA0041F9A: "glyph_raster",
    GET_LANG: "get_lang",
    GET_CM: "get_cm",
    0xA0167946: "set_cm_raw",
    0xA016F86E: "cm_setter",
    GET_EXPO_MODE: "get_expo_mode",
    0xA016F9EC: "expo_setter",
    0xA01675DA: "expo_program",
    NV_LOAD: "nv_load",
    NV_FLUSH: "nv_flush",
    NV_RESET: "nv_reset",
    NOR_READ: "nor_read",
    NOR_WRITE: "nor_write",
    MALLOC: "malloc",
    FREE: "free",
    0xA000EE68: "memcpy",
    0xA000EEBA: "memset",
    0xA0173358: "__ARM_common_switch8",
    BKT_COUNT: "bkt_count",
    BKT_EV_Q16: "bkt_ev_q16",
    0xA0167B3C: "get_bkt_ord",
    0xA003A180: "lv_config",
    LV_REFRESH_DIRTY: "lv_refresh",
    0xA003989C: "lv_mono",
    0xA0039CE2: "lv_cmcm",
    0xA0039864: "lv_gamma",
    0xA009D940: "b2y_set_gamma",
    0xA009C334: "b2y_set_mat",
    0xA01633CA: "get_lv_cc7",
    0xA016339A: "get_lv_ccn",
    0xA0044392: "qs_screen",
    0xA00585C8: "menu_screen",
    0xA005F818: "updater_screen",
    0xA01657E4: "heap_bounds",
}
