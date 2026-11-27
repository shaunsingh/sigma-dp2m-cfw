// stock M6M entry points as absolute function symbols; bit 0 set = Thumb (lld picks BL vs BLX from it)

.macro stock name, addr
    .globl \name
    .type \name, %function
    .set \name, \addr
.endm

stock stock_get_cm,        0xA0167927
stock stock_get_grid,      0xA0167F99
stock stock_get_bkt_ord,   0xA0167B3D
stock stock_get_expo_mode, 0xA0167893
stock stock_get_menu_6b,   0xA004CD8D
stock stock_ui_screen_id,  0xA004333D
stock stock_lv_active,     0xA00344F3
stock stock_lv_refresh,    0xA003A261
stock stock_lv_mono,       0xA003989D
stock stock_lv_cmcm,       0xA0039CE3
stock stock_get_lv_cc7,    0xA01633CB
stock stock_get_lv_ccn,    0xA016339B
stock stock_grid_draw,     0xA004C3ED
stock stock_fill_rect,     0xA00419A7
stock stock_flush_rect,    0xA0041C7B
stock stock_accel_x,       0xA001A38F
stock stock_accel_y,       0xA001A3A3
stock stock_b2y_set_mat,   0xA009C334
stock stock_b2y_set_gamma, 0xA009D940
stock stock_icon,          0xA0042AAD
stock stock_nor_write,     0xA00206FB
stock stock_cur_backup,    0xA01675B7
stock stock_cur_restore,   0xA01675C9
stock stock_cur_apply,     0xA016F963
stock stock_slot_same,     0xA0167619
stock stock_still_apply,   0xA003D9FB
stock stock_movie_apply,   0xA003E85B
stock stock_get_iso,       0xA016F5AF
stock stock_lv_status_cm,  0xA004C7BD
stock stock_pb_info_cm,    0xA0047735
stock stock_text,          0xA0042685
stock stock_font_scale,    0xA004226D
stock stock_disp_init,     0xA0168D0D
stock stock_screen_post,   0xA0042AED
stock stock_screen_goto,   0xA0042B7B
stock stock_osd_clear,     0xA00416A9
stock stock_osd_commit,    0xA004190F
stock stock_argb_clear,    0xA0041A61
stock stock_argb_commit,   0xA0041B49
