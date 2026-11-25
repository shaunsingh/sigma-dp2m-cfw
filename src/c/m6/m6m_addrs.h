#ifndef DP2M_M6M_ADDRS_H
#define DP2M_M6M_ADDRS_H

// m6m (arm926ej-s BE, S3) symbols & cave map
#define SYM_UI_STR_LOOKUP    0xA0044AAC
#define SYM_UI_STR_HOOKPT    0xA0044AAE
#define SYM_GET_MENU_TBL     0xA004CD08
#define SYM_GET_MENU_6B      0xA004CD8C

#define SYM_GET_CM           0xA0167926
#define SYM_GET_GRID         0xA0167F98
#define SYM_GET_BKT_ORD      0xA0167B3C
#define SYM_GET_BKT_EV_Q16   0xA0002FE4
#define SYM_BKT_STEP_IDX     0xA07FBE5A
#define SYM_BKT_STEP_TBL     0xA0176F14

#define SYM_AE_AUTOISO_A     0xA000D83E
#define SYM_AE_AUTOISO_P1    0xA000D9A2
#define SYM_AE_AUTOISO_P2    0xA000D9E8

#define SYM_LV_GAMMA_CALL    0xA0039890
#define SYM_B2Y_SET_GAMMA    0xA009D940
#define SYM_LV_CMCM_CALL     0xA003A20A
#define SYM_LV_CMCM_FN       0xA0039CE2
#define SYM_GET_LV_CC_7      0xA01633CA
#define SYM_GET_LV_CC_N      0xA016339A
#define SYM_B2Y_SET_MAT      0xA009C334
#define SYM_LV_MONO_CALL     0xA003A1E6
#define SYM_LV_MONO_FN       0xA003989C
#define SYM_LV_REFRESH       0xA003A260
#define SYM_LV_YCAP_FN       0xA0033F1A
#define SYM_LV_YCAP_LIT      0xA07B06F8

#define SYM_OSD_ICON_CALL    0xA016BC4E
#define SYM_GRID_DRAW_FN     0xA004C3EC
#define SYM_UI_SCREEN_ID     0xA004333C
#define SYM_LV_ACTIVE        0xA00344F2
#define SYM_FILL_RECT        0xA00419A6
#define SYM_FLUSH_RECT       0xA0041C7A
#define SYM_ACCEL_X          0xA001A38E
#define SYM_ACCEL_Y          0xA001A3A2
#define SYM_ARGB_FB          0xA21D8C00

// nvram settings & volatile lv ptrs inside unused 36B header 0xA0939688..AB (well below lang 0xA0939776)
#define VA_NVRAM_PAPER       0xA0939688
#define VA_NVRAM_FX          0xA0939689
#define VA_NVRAM_FILL        0xA093968A
#define VA_NVRAM_MINSHUTTER  0xA093968B
#define VA_NVRAM_PEAKCOL     0xA093968C

#define VA_RAM_LV_FCNT       0xA0939694
#define VA_RAM_LIVE_Y_PTR    0xA0939698
#define VA_RAM_LIVE_CB_PTR   0xA093969C
#define VA_RAM_LIVE_CR_PTR   0xA09396A0

// cave 1 (factory screens 33..42)
#define VA_SAFE_RET0         0xA005A400
#define VA_ISO50_HOOK_1      0xA005A408
#define VA_ISO50_HOOK_2      0xA005A41C
#define VA_STR_HOOK          0xA005A430
#define VA_LV_MONO_HOOK      0xA005A464
#define VA_OSD_ICON_HOOK     0xA005A488
#define VA_LV_CMCM_TRAMP     0xA005A4B4
#define VA_LV_GAMMA_WRAP     0xA005A4D4
#define VA_GET_MENU_HOOK     0xA005A4E8
#define VA_LV_YCAP_HOOK      0xA005A5A4
#define VA_IPC_CMD_HOOK      0xA005A5D0
#define VA_GET_PAPER         0xA005A670
#define VA_SET_PAPER         0xA005A67C
#define VA_GET_FX            0xA005A694
#define VA_SET_FX            0xA005A6A0
#define VA_GET_FILL          0xA005A6AC
#define VA_SET_FILL          0xA005A6B8
#define VA_GET_MINSH         0xA005A6C4
#define VA_SET_MINSH         0xA005A6D0
#define VA_GET_PEAKC         0xA005A6DC
#define VA_SET_PEAKC         0xA005A6E8
#define VA_BKT5_HOOK         0xA005A6F4
#define VA_AUTOISO_A_HOOK    0xA005A768
#define VA_AUTOISO_P_HOOK    0xA005A790
#define VA_LV_CMCM_C         0xA005A7C0
#define VA_LV_GAMMA_C        0xA005A900
#define VA_GRID_WGET_C       0xA005AA60
#define VA_GRID_DRAW_TRAMP   0xA005ABA0
#define VA_GRID_DRAW_C       0xA005ABD0
#define VA_AUTO_PAPER_TBL    0xA005B7D0
#define VA_MONO_Q7_TBL       0xA005B7E0
#define VA_TV_MIN_TBL        0xA005B844
#define VA_STR_PTRS_83       0xA005B860

// cave 2S (factory screens 43..46)
#define VA_CMCM_PACKED_30    0xA005BE40
#define VA_LV_LUTS_20        0xA005C060
#define VA_LV_LUTS_BW_5      0xA005C4A0
#define VA_COLOR_MENU_44     0xA005C500
#define VA_GRID_MENU_17      0xA005CAA0

// factory service menu cave
#define VA_ISO_MENU_2        0xA07EC9CC
#define VA_ISO_MENU_1        0xA07ECC8C
#define VA_PAPER_MENU        0xA07ECDCC
#define VA_MINSHUTTER_MENU   0xA07ECF0C
#define VA_SHOOT_P3_NEW      0xA07ED00C
#define VA_SETUP_P2_V5       0xA07ED0EC
#define VA_BKTORD_MENU_6     0xA07ED1CC
#define VA_FILMFX_MENU       0xA07ED2AC
#define VA_SETUP_P5_NEW      0xA07ED36C
#define VA_PEAKCOL_MENU      0xA07ED40C
#define VA_X3FILL_MENU       0xA07ED4AC
#define VA_ISO_AUTO_3        0xA07ED54C

#endif
