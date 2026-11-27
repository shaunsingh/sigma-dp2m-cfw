//! Stock S3 virtual addresses for DP2M v1.05 M6M (shared across verifier, RE, and sim).

pub const GET_MENU_TABLE: u32 = 0xA004_CD08;
pub const UI_STRING: u32 = 0xA004_4AAC;
pub const STR_TBL: u32 = 0xA07D_F144;
pub const STR_STRIDE: u32 = 0x44;
pub const STR_LANGS: u32 = 17;
pub const STOCK_SIDS: u32 = 672;
pub const SCREEN_VT: u32 = 0xA07D_EB28;
pub const SCREEN_POST: u32 = 0xA004_2AEC;
pub const ICON_GET: u32 = 0xA004_2AAC;
pub const ICON_RESULT: u32 = 0xA07D_D77C;
pub const ICON_TBL: u32 = 0xA07D_D784;
pub const ICON_MAX: u32 = 0x1A1;
pub const CLUT: u32 = 0xA07D_EC54;
pub const CLUT_N: usize = 0xDB;
pub const ICON_HI: u8 = 0xC8;
pub const OSD: u32 = 0xA213_0000;
pub const ARGB_FB: u32 = 0xA21D_8C00;
pub const OSD_DIRTY: u32 = 0xA07D_D760;
pub const BLIT: u32 = 0xA004_0DD8;
pub const FILL_BOX: u32 = 0xA004_1198;
pub const DMA_2D_COPY_MODE: u32 = 0xA00B_125C;
pub const DMA_2D_COPY: u32 = 0xA00B_14A0;
pub const DMA_RECT_FILL: u32 = 0xA00B_1870;
pub const DMA_MEM_COPY: u32 = 0xA00B_23A8;
pub const QS_STATE: u32 = 0xA07D_F008;
pub const QS_SLOT_ROWS: u32 = 0xA004_35D8;
pub const QS_STRIP: u32 = 0xA004_3E5C;
pub const QS_STRIP_CHECK: u32 = 0xA004_3E9C;
pub const MENU_ROW_COUNT: u32 = 0xA004_D4D6;
pub const ROW_OSD_ICON: u32 = 0xA004_4FCC;
pub const LV_STATUS_CM: u32 = 0xA004_C7BC;
pub const LV_INFO_CM: u32 = 0xA016_BC4A;
pub const STRIP_BG_CLEAR: u32 = 0xA004_4E48;

pub const TEXT_DRAW: u32 = 0xA004_2684;
pub const LINE_BUF: u32 = 0xA093_A514;
pub const LINE_W: usize = 640;
pub const LINE_ROWS: usize = 256;
pub const FONT_SCALE: u32 = 0xA07D_D774;
pub const GLYPH_RASTER: (u32, u32) = (0xA230_4C00, 0xA230_8C00);
pub const FONT_WORK: (u32, u32) = (0xA234_6400, 0xA236_6400);
pub const GET_LANG: u32 = 0xA016_7DD4;
pub const UITRON: (u32, u32) = (0xA007_0000, 0xA007_6000);

pub const NV_BLOCK: u32 = 0xA093_9684;
pub const NV_LEN: usize = 0x44C;
pub const NV_PERSIST: usize = 0x3C4;
pub const NV_DEFAULTS: u32 = 0xA07F_9E68;
pub const NV_CM: u32 = 0xA093_96C3;
pub const NV_GRID: u32 = 0xA093_96FB;
pub const NV_BKT_ORD: u32 = 0xA093_96DF;
pub const NV_ISO_STEP: u32 = 0xA093_96DD;
pub const MENU_ISO: u32 = 0x01;
pub const NV_EXPO: u32 = 0xA093_96AC;
pub const NV_CUR: u32 = 0xA093_96C0;
pub const NV_LOAD: u32 = 0xA004_0110;
pub const NV_FLUSH: u32 = 0xA004_017A;
pub const NV_RESET: u32 = 0xA016_7574;
pub const NOR_READ: u32 = 0xA002_06CC;
pub const NOR_WRITE: u32 = 0xA002_06FA;
pub const MALLOC: u32 = 0xA003_D9C0;
pub const FREE: u32 = 0xA003_D9AC;
pub const CALLOC: u32 = 0xA003_D990;
pub const REALLOC: u32 = 0xA003_D9D8;

pub const AE_SENSOR_EV100: u32 = 0xA000_273C;
pub const AE_ENABLE_LV_METER: u32 = 0xA000_2CB2;
pub const AE_SOLVE_CORE: u32 = 0xA000_D6B6;
pub const AE_AV_TO_Q16: u32 = 0xA000_DDF6;
pub const AE_TV_TO_Q16: u32 = 0xA000_DE26;
pub const AE_SOLVE_UI: u32 = 0xA003_EABE;

pub const PB_SCREEN: u32 = 0xA005_75E2;
pub const PB_SELECT_VBUF: u32 = 0xA000_F222;
pub const PB_OSD_DISPATCH: u32 = 0xA005_300E;
pub const PB_STEP_OR_CUR: u32 = 0xA005_3354;
pub const PB_DECODE_RESAMPLE: u32 = 0xA005_3F5C;
pub const PB_ZOOM_CORE: u32 = 0xA005_44A8;
pub const PB_YCBCR_TO_RGB_PLANAR: u32 = 0xA005_4AB2;
pub const PB_IIP_HIST: u32 = 0xA004_A7AC;
pub const PB_HIST_BAR_DRAW: u32 = 0xA004_8F7C;
pub const DCF_GET_FILE_INFO: u32 = 0xA011_51D0;
pub const DCF_COUNT_IMAGES: u32 = 0xA013_4234;
pub const DCF_READ_JPEG_EXIF: u32 = 0xA013_42EC;
pub const DCF_READ_X3F_EXIF: u32 = 0xA013_4326;

pub const GET_CM: u32 = 0xA016_7926;
pub const GET_GRID: u32 = 0xA016_7F98;
pub const GET_EXPO_MODE: u32 = 0xA016_7892;
pub const BKT_COUNT: u32 = 0xA016_8100;
pub const BKT_EV_Q16: u32 = 0xA000_2FE4;
pub const BKT_STEP_IDX: u32 = 0xA07F_BE5A;
pub const BKT_STEP_TBL: u32 = 0xA017_6F14;
pub const AUTOISO_A: u32 = 0xA000_D83E;
pub const AUTOISO_P: [u32; 2] = [0xA000_D9A2, 0xA000_D9E8];
pub const IPC_CMD20: u32 = 0xA003_60A2;
pub const LV_YCAP: u32 = 0xA003_3F1A;
pub const LV_REFRESH_DIRTY: u32 = 0xA003_A260;
pub const LV_GAMMA_SITE: u32 = 0xA003_9890;
pub const LV_MONO_SITE: u32 = 0xA003_A1E6;
pub const LV_CMCM_SITE: u32 = 0xA003_A20A;

pub const VERSION_IMM: u32 = 0xA005_F87C;
pub const UPDATER_BANNERS: [u32; 3] = [0xA005_FC3C, 0xA005_FC5C, 0xA005_FC7C];

pub const NAMES: &[(u32, &str)] = &[
    (GET_MENU_TABLE, "get_menu_table"),
    (UI_STRING, "ui_string"),
    (SCREEN_POST, "screen_post"),
    (ICON_GET, "icon_get"),
    (BLIT, "osd_blit"),
    (TEXT_DRAW, "text_draw"),
    (0xA004_2274, "text_measure"),
    (0xA004_1F9A, "glyph_raster"),
    (GET_LANG, "get_lang"),
    (GET_CM, "get_cm"),
    (0xA016_7946, "set_cm_raw"),
    (0xA016_F86E, "cm_setter"),
    (GET_EXPO_MODE, "get_expo_mode"),
    (0xA016_F9EC, "expo_setter"),
    (0xA016_75DA, "expo_program"),
    (NV_LOAD, "nv_load"),
    (NV_FLUSH, "nv_flush"),
    (NV_RESET, "nv_reset"),
    (NOR_READ, "nor_read"),
    (NOR_WRITE, "nor_write"),
    (MALLOC, "malloc"),
    (FREE, "free"),
    (0xA000_EE68, "memcpy"),
    (0xA000_EEBA, "memset"),
    (0xA017_3358, "__ARM_common_switch8"),
    (BKT_COUNT, "bkt_count"),
    (BKT_EV_Q16, "bkt_ev_q16"),
    (0xA016_7B3C, "get_bkt_ord"),
    (0xA003_A180, "lv_config"),
    (LV_REFRESH_DIRTY, "lv_refresh"),
    (0xA003_989C, "lv_mono"),
    (0xA003_9CE2, "lv_cmcm"),
    (0xA003_9864, "lv_gamma"),
    (0xA009_D940, "b2y_set_gamma"),
    (0xA009_C334, "b2y_set_mat"),
    (0xA016_33CA, "get_lv_cc7"),
    (0xA016_339A, "get_lv_ccn"),
    (0xA004_4392, "qs_screen"),
    (PB_SCREEN, "pb_screen"),
    (0xA005_85C8, "menu_screen"),
    (0xA005_F818, "updater_screen"),
    (0xA016_57E4, "heap_bounds"),
    (CALLOC, "calloc"),
    (REALLOC, "realloc"),
    (DMA_2D_COPY_MODE, "dma_2d_copy_mode"),
    (DMA_2D_COPY, "dma_2d_copy"),
    (DMA_RECT_FILL, "dma_rect_fill"),
    (DMA_MEM_COPY, "dma_mem_copy"),
    (AE_SENSOR_EV100, "ae_sensor_ev100"),
    (AE_ENABLE_LV_METER, "ae_enable_lv_meter"),
    (AE_SOLVE_CORE, "ae_solve_core"),
    (AE_SOLVE_UI, "ae_solve_ui"),
    (AE_AV_TO_Q16, "ae_av_to_q16"),
    (AE_TV_TO_Q16, "ae_tv_to_q16"),
    (PB_SELECT_VBUF, "pb_select_vbuf"),
    (PB_OSD_DISPATCH, "pb_osd_dispatch"),
    (PB_STEP_OR_CUR, "pb_step_or_cur"),
    (PB_DECODE_RESAMPLE, "pb_decode_resample"),
    (PB_ZOOM_CORE, "pb_zoom_core"),
    (PB_YCBCR_TO_RGB_PLANAR, "pb_ycbcr_to_rgb_planar"),
    (PB_IIP_HIST, "pb_iip_hist"),
    (PB_HIST_BAR_DRAW, "pb_hist_bar_draw"),
    (DCF_GET_FILE_INFO, "dcf_get_file_info"),
    (DCF_COUNT_IMAGES, "dcf_count_images"),
    (DCF_READ_JPEG_EXIF, "dcf_read_jpeg_exif"),
    (DCF_READ_X3F_EXIF, "dcf_read_x3f_exif"),
    (0xA007_1830, "get_mpf"),
    (0xA007_1970, "pget_mpf"),
    (0xA007_1B00, "rel_mpf"),
    (0xA007_1C54, "tget_mpf"),
    (0xA007_1F30, "get_mpl"),
    (0xA007_212C, "pget_mpl"),
    (0xA007_22A0, "tget_mpl"),
    (0xA007_2388, "rel_mpl"),
    (0xA007_2C74, "ref_mbx"),
    (0xA007_3B70, "get_tid"),
    (0xA007_52D4, "ref_tsk"),
];
