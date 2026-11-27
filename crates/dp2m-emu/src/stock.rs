//! Stock S3 virtual addresses for DP2M v1.05 M6M (shared across verifier, RE, and sim).

pub const GET_MENU_TABLE: u32 = 0xA004_CD08;
pub const UI_STRING: u32 = 0xA004_4AAC;
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

// Simulator call sites and data not part of the public stock map.
pub(crate) const GET_MPF: u32 = 0xA007_1830;
pub(crate) const PGET_MPF: u32 = 0xA007_1970;
pub(crate) const TGET_MPF: u32 = 0xA007_1C54;
pub(crate) const REL_MPF: u32 = 0xA007_1B00;
pub(crate) const REF_MPF: u32 = 0xA007_1A3C;
pub(crate) const GET_MPL: u32 = 0xA007_1F30;
pub(crate) const PGET_MPL: u32 = 0xA007_212C;
pub(crate) const REF_MPL: u32 = 0xA007_22A0;
pub(crate) const REL_MPL: u32 = 0xA007_2388;
pub(crate) const POL_FLG: u32 = 0xA007_2870;
pub(crate) const WAI_FLG: u32 = 0xA007_37B8;
pub(crate) const PRCV_MBX: u32 = 0xA007_29B8;
pub(crate) const RCV_MBX: u32 = 0xA007_2A88;
pub(crate) const TRCV_MBX: u32 = 0xA007_3240;
pub(crate) const REF_FLG: u32 = 0xA007_2BCC;
pub(crate) const REF_MBX: u32 = 0xA007_2C74;
pub(crate) const REF_SEM: u32 = 0xA007_2D24;
pub(crate) const GET_TID: u32 = 0xA007_3B70;
pub(crate) const REF_SYS: u32 = 0xA007_3BDC;
pub(crate) const REF_ALM: u32 = 0xA007_4808;
pub(crate) const REF_CYC: u32 = 0xA007_48F4;
pub(crate) const STA_CYC: u32 = 0xA007_4AC0;
pub(crate) const STP_CYC: u32 = 0xA007_4C50;
pub(crate) const GET_PRI: u32 = 0xA007_51F0;
pub(crate) const REF_TSK: u32 = 0xA007_52D4;
pub(crate) const LIBC_MEMCPY: u32 = 0xA010_4DAC;
pub(crate) const LIBC_MEMSET: u32 = 0xA010_4DB8;
pub(crate) const LIBC_AEABI_MEMSET: u32 = 0xA010_4DD4;
pub(crate) const LIBC_AEABI_MEMMOVE: u32 = 0xA010_4EB8;
pub(crate) const LIBC_AEABI_MEMCLR: u32 = 0xA010_4F8C;
pub(crate) const LIBC_AEABI_MEMCLR4: u32 = 0xA010_5030;
pub(crate) const UI_INIT_A: u32 = 0xA004_0D74;
pub(crate) const UI_INIT_B: u32 = 0xA004_0D7C;
pub(crate) const SCREEN_POST_MBOX: u32 = 0xA004_2B4E;
pub(crate) const SCREEN_POST_RET: u32 = 0xA004_2B70;
pub(crate) const UI_DISPATCH: u32 = 0xA004_3234;
pub(crate) const UI_SCREEN_ID: u32 = 0xA004_333C;
pub(crate) const SET_CARD_READY: u32 = 0xA016_80A0;
pub(crate) const LV_ACTIVE: u32 = 0xA003_44F2;
pub(crate) const ACCEL_X: u32 = 0xA001_A38E;
pub(crate) const ACCEL_Y: u32 = 0xA001_A3A2;
pub(crate) const AE_STATE: u32 = 0xA080_08F8;
pub(crate) const AE_SET_MODE: u32 = 0xA000_D1A2;
pub(crate) const AE_SET_AUTO_ISO_MIN: u32 = 0xA000_D2AA;
pub(crate) const AE_SET_AUTO_ISO_MAX: u32 = 0xA000_D2B2;
pub(crate) const AE_SYNC_ISO: u32 = 0xA003_F3A6;
pub(crate) const AE_MAP_ISO_IDX: u32 = 0xA003_F288;
pub(crate) const AE_SYNC_TV: u32 = 0xA003_EDD0;
pub(crate) const AE_SYNC_AV: u32 = 0xA003_EECC;
pub(crate) const AE_SYNC_EV: u32 = 0xA003_EC88;
pub(crate) const AE_OUT_AV_IDX: u32 = 0xA07D_D6E0;
pub(crate) const AE_OUT_TV_IDX: u32 = 0xA07D_D6E4;
pub(crate) const GET_AUTO_ISO_MIN_UI: u32 = 0xA016_F4FE;
pub(crate) const GET_AUTO_ISO_MAX_UI: u32 = 0xA016_F556;
pub(crate) const GET_ISO_IDX: u32 = 0xA016_F5AE;
pub(crate) const GET_EV_IDX: u32 = 0xA016_7BF0;
pub(crate) const LV_CONFIG: u32 = 0xA003_A180;
pub(crate) const LV_MARK_SHARP_DIRTY: u32 = 0xA003_A258;
pub(crate) const LV_MARK_CONTRAST_DIRTY: u32 = 0xA003_A268;
pub(crate) const LV_YCAP_END: u32 = 0xA003_3F20;
pub(crate) const GET_LV_CC7: u32 = 0xA016_33CA;
pub(crate) const GET_LV_CCN: u32 = 0xA016_339A;
pub(crate) const B2Y_SET_MAT: u32 = 0xA009_C334;
pub(crate) const B2Y_SET_MONO: u32 = 0xA009_CD7C;
pub(crate) const B2Y_SET_CHROMA: u32 = 0xA009_CF84;
pub(crate) const B2Y_SET_SHARP_LUT: u32 = 0xA009_D650;
pub(crate) const B2Y_SET_GAMMA: u32 = 0xA009_D940;
pub(crate) const GET_COLOR_SPACE: u32 = 0xA016_797E;
pub(crate) const IPC_CMD20_END: u32 = 0xA003_60A8;
pub(crate) const MN_CM_SITE: u32 = 0xA014_8CF8;
pub(crate) const MN_CM_END: u32 = 0xA014_8D2E;
pub(crate) const PB_CM_PARSE_START: u32 = 0xA014_78A6;
pub(crate) const PB_CM_PARSE_END: u32 = 0xA014_793C;
pub(crate) const PB_HW_JPEG_DEC: u32 = 0xA005_35C8;
pub(crate) const PB_OPEN_JPEG_B: u32 = 0xA005_7D40;
pub(crate) const PB_OPEN_JPEG_A: u32 = 0xA005_7DE6;
pub(crate) const PB_DEC_INIT: u32 = 0xA005_7EA8;
pub(crate) const PB_OPEN_IMAGE_B: u32 = 0xA005_7EB0;
pub(crate) const PB_OPEN_IMAGE_A: u32 = 0xA005_7F16;
pub(crate) const DCF_DIR_COUNTS: u32 = 0xA096_2758;
pub(crate) const DCF_SYNC_CUR: u32 = 0xA011_49EC;
pub(crate) const SD_MOUNT_STATUS: u32 = 0xA011_6A4C;
pub(crate) const DCF_GET_ENTRY: u32 = 0xA013_4286;
pub(crate) const DCF_SCAN_READY: u32 = 0xA014_FDE6;
pub(crate) const DCF_GET_CUR: u32 = 0xA015_39BE;
pub(crate) const DCF_GET_TOTAL: u32 = 0xA015_39DE;
pub(crate) const DCF_STEP_NEXT: u32 = 0xA015_3C22;
pub(crate) const DCF_STEP_PREV: u32 = 0xA015_3C7E;
pub(crate) const DCF_SET_CUR: u32 = 0xA015_3CE6;
pub(crate) const SD_FREE_SPACE: u32 = 0xA015_5144;
pub(crate) const PB_VBUF_DEFAULT_VA: u32 = 0xA235_E600;
pub(crate) const NV_DISPLAY_MODE: u32 = 0xA093_9698;
pub(crate) const NV_WB: u32 = 0xA093_96C4;
pub(crate) const NV_AUTO_ISO_MAX: u32 = 0xA093_96DC;
pub(crate) const NV_SHARP: u32 = 0xA093_96D4;
pub(crate) const NV_CONTRAST: u32 = 0xA093_96D5;
pub(crate) const NV_SAT: u32 = 0xA093_96D6;
pub(crate) const AE_SET_ISO_STEP: u32 = 0xA016_F5DE;
pub(crate) const SET_CM: u32 = 0xA016_F86E;
pub(crate) const SET_GRID: u32 = 0xA016_7FA0;
pub(crate) const NV_LUT_SLOT: u32 = 0xA093_9690;
pub(crate) const NV_LUT_ENABLE: u32 = 0xA093_9689;
pub(crate) const PB_CUR_INDEX: u32 = 0xA07F_220C;
pub(crate) const DIAG_LOG_A: u32 = 0xA010_EF28;
pub(crate) const DIAG_LOG_B: u32 = 0xA010_EF48;
pub(crate) const REC_COUNTER_SET: u32 = 0xA001_B020;
pub(crate) const REC_COUNTER_GET: u32 = 0xA001_B026;
pub(crate) const NV_SYNC: u32 = 0xA014_3336;
pub(crate) const NV_BUSY: u32 = 0xA014_5650;
pub(crate) const PB_VBUFS: [u32; 5] = [
    PB_VBUF_DEFAULT_VA, 0xA243_F600, 0xA252_0600, 0xA260_1600, 0xA259_0E00,
];

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
    (SET_CM, "cm_setter"),
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
    (LV_CONFIG, "lv_config"),
    (LV_REFRESH_DIRTY, "lv_refresh"),
    (0xA003_989C, "lv_mono"),
    (0xA003_9CE2, "lv_cmcm"),
    (0xA003_9864, "lv_gamma"),
    (B2Y_SET_GAMMA, "b2y_set_gamma"),
    (B2Y_SET_MAT, "b2y_set_mat"),
    (GET_LV_CC7, "get_lv_cc7"),
    (GET_LV_CCN, "get_lv_ccn"),
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
    (GET_MPF, "get_mpf"),
    (PGET_MPF, "pget_mpf"),
    (REL_MPF, "rel_mpf"),
    (TGET_MPF, "tget_mpf"),
    (GET_MPL, "get_mpl"),
    (PGET_MPL, "pget_mpl"),
    (REF_MPL, "tget_mpl"), // Legacy trace label; stock writes T_RMPL.
    (REL_MPL, "rel_mpl"),
    (REF_MBX, "ref_mbx"),
    (GET_TID, "get_tid"),
    (REF_TSK, "ref_tsk"),
];
