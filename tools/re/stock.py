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
ARGB_FB = 0xA21D8C00  # 640x480 RGBA4444 overlay plane (LV uses rows 0..423)
OSD_DIRTY = 0xA07DD760  # dirty rects merged by the OSD flush
BLIT = 0xA0040DD8  # (src, x, y, w, [sp] h, hi, lo, transparent, mode)
FILL_BOX = 0xA0041198
DMA_2D_COPY_MODE = 0xA00B125C  # (mode, src, src_stride, dst_stride, [sp] dst, w_bytes, rows)
DMA_2D_COPY = 0xA00B14A0  # (src, src_stride, dst_stride, dst, [sp] w_bytes, rows)
DMA_RECT_FILL = 0xA00B1870  # (mode, fill, stride, dst, [sp] w_bytes, rows)
DMA_MEM_COPY = 0xA00B23A8  # (dst, src, len)
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
GLYPH_RASTER = (0xA2304C00, 0xA2308C00)
FONT_WORK = (0xA2346400, 0xA2366400)  # engine workspace (literal at 0xA00421C4)
GET_LANG = 0xA0167DD4
UITRON = (0xA0070000, 0xA0076000)  # kernel service calls

# settings & NVRAM
NV_BLOCK, NV_LEN, NV_PERSIST = 0xA0939684, 0x44C, 0x3C4
NV_DEFAULTS = 0xA07F9E68
NV_CM = 0xA09396C3
NV_GRID = 0xA09396FB
NV_BKT_ORD = 0xA09396DF
NV_ISO_STEP = 0xA09396DD  # 1: 1/3 EV ISO menu
MENU_ISO = 0x01
NV_EXPO = 0xA09396AC
NV_CUR = 0xA09396C0  # shooting settings (0x8C B) a custom slot stores
NV_LOAD = 0xA0040110  # (dst) boot load of NOR bank 0
NV_FLUSH = 0xA004017A  # NVRAM -> NOR bank 0 record at 8 (stock block at 0x20)
NV_RESET = 0xA0167574
NOR_READ, NOR_WRITE = 0xA00206CC, 0xA00206FA  # (off, n, buf), bank 0
MALLOC, FREE, CALLOC, REALLOC = 0xA003D9C0, 0xA003D9AC, 0xA003D990, 0xA003D9D8

# native AE solver & photometry
AE_SENSOR_EV100 = 0xA000273C
AE_ENABLE_LV_METER = 0xA0002CB2
AE_SET_MODE = 0xA000D1A2
AE_SET_AUTO_ISO_MIN, AE_SET_AUTO_ISO_MAX = 0xA000D2AA, 0xA000D2B2
AE_SOLVE_CORE = 0xA000D6B6
AE_AV_TO_Q16, AE_TV_TO_Q16 = 0xA000DDF6, 0xA000DE26
AE_SOLVE_UI = 0xA003EABE
AE_SYNC_EV, AE_SYNC_TV, AE_SYNC_AV, AE_SYNC_ISO = 0xA003EC88, 0xA003EDD0, 0xA003EECC, 0xA003F3A6
AE_MAP_ISO_IDX = 0xA003F288
AE_STATE = 0xA08008F8
AE_OUT_AV_IDX, AE_OUT_TV_IDX = 0xA07DD6E0, 0xA07DD6E4
GET_AUTO_ISO_MIN_UI, GET_AUTO_ISO_MAX_UI, GET_ISO_IDX = 0xA016F4FE, 0xA016F556, 0xA016F5AE

# SCREEN_PB (0x19) & DCF
PB_SCREEN = 0xA00575E2
PB_STATE = 0xA07F21D8  # +0x34 current 1-based DCF index, +0x3C total count
PB_SELECT_VBUF = 0xA000F222  # 1->0xA235E600, 2->0xA243F600, 4->0xA2520600, 8->0xA2601600
PB_OSD_DISPATCH = 0xA005300E
PB_STEP_OR_CUR = 0xA0053354
PB_DECODE_RESAMPLE = 0xA0053F5C
PB_ZOOM_CORE = 0xA00544A8
PB_YCBCR_TO_RGB_PLANAR = 0xA0054AB2
PB_IIP_HIST = 0xA004A7AC
PB_HIST_BAR_DRAW = 0xA0048F7C
DCF_DIR_COUNTS = 0xA0962758  # 999 x u32 (low u16 = count per DCF folder 100..999)
DCF_GET_FILE_INFO = 0xA01151D0
DCF_COUNT_IMAGES = 0xA0134234
DCF_READ_JPEG_EXIF, DCF_READ_X3F_EXIF = 0xA01342EC, 0xA0134326
GET_CM = 0xA0167926
GET_GRID = 0xA0167F98
GET_EXPO_MODE = 0xA0167892
BKT_COUNT = 0xA0168100
BKT_EV_Q16 = 0xA0002FE4  # (shot) -> EV offset q16
BKT_STEP_IDX, BKT_STEP_TBL = 0xA07FBE5A, 0xA0176F14  # EV step index -> q16 step
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
    PB_SCREEN: "pb_screen",
    0xA00585C8: "menu_screen",
    0xA005F818: "updater_screen",
    0xA01657E4: "heap_bounds",
    CALLOC: "calloc",
    REALLOC: "realloc",
    DMA_2D_COPY_MODE: "dma_2d_copy_mode",
    DMA_2D_COPY: "dma_2d_copy",
    DMA_RECT_FILL: "dma_rect_fill",
    DMA_MEM_COPY: "dma_mem_copy",
    AE_SENSOR_EV100: "ae_sensor_ev100",
    AE_ENABLE_LV_METER: "ae_enable_lv_meter",
    AE_SOLVE_CORE: "ae_solve_core",
    AE_SOLVE_UI: "ae_solve_ui",
    AE_AV_TO_Q16: "ae_av_to_q16",
    AE_TV_TO_Q16: "ae_tv_to_q16",
    PB_SELECT_VBUF: "pb_select_vbuf",
    PB_OSD_DISPATCH: "pb_osd_dispatch",
    PB_STEP_OR_CUR: "pb_step_or_cur",
    PB_DECODE_RESAMPLE: "pb_decode_resample",
    PB_ZOOM_CORE: "pb_zoom_core",
    PB_YCBCR_TO_RGB_PLANAR: "pb_ycbcr_to_rgb_planar",
    PB_IIP_HIST: "pb_iip_hist",
    PB_HIST_BAR_DRAW: "pb_hist_bar_draw",
    DCF_GET_FILE_INFO: "dcf_get_file_info",
    DCF_COUNT_IMAGES: "dcf_count_images",
    DCF_READ_JPEG_EXIF: "dcf_read_jpeg_exif",
    DCF_READ_X3F_EXIF: "dcf_read_x3f_exif",
    0xA0071830: "get_mpf",
    0xA0071970: "pget_mpf",
    0xA0071B00: "rel_mpf",
    0xA0071C54: "tget_mpf",
    0xA0071F30: "get_mpl",
    0xA007212C: "pget_mpl",
    0xA00722A0: "tget_mpl",
    0xA0072388: "rel_mpl",
    0xA0072C74: "ref_mbx",
    0xA0073B70: "get_tid",
    0xA00752D4: "ref_tsk",
}
