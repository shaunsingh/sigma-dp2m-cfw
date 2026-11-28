// register-exact trampolines for stock call sites; everything else lives in C
    .syntax unified
    .thumb

.macro hook kind, site, target, expect
    .pushsection .dp2m.hooks, "a", %progbits
    .word \kind, \site, \target, \expect
    .popsection
.endm
.equ HOOK_CALL, 1
.equ HOOK_BL, 2
.equ HOOK_WORD, 3
.equ HOOK_HALF, 4

.macro fn name
    .section .text.\name, "ax", %progbits
    .globl \name
    .type \name, %function
    .thumb_func
\name:
.endm

// stub for retired factory screen vtable slots
fn dp2m_ret0
    movs r0, #0
    bx   lr

// iso 1-EV clamp 0xA003F2CC: `movs r1,r0; push {lr}; movs r0,#5` -> `push {lr}; bl`; ISO 50 (1) passes through
fn iso50_clamp_1
    cmp  r0, #1
    bne  1f
    pop  {pc}
1:  movs r1, r0
    movs r0, #5
    bx   lr
    hook HOOK_HALF, 0xA003F2CC, 0xB500, 0x0001
    hook HOOK_BL, 0xA003F2CE, iso50_clamp_1, 0xB5002005

// 0xA003F30A (next 1-EV step up when bumping upper limit 0xD3): stock default returns r1 (1); map 1 -> 5 (ISO 200)
fn iso50_clamp_2
    cmp  r0, #1
    bne  1f
    movs r0, #5
    pop  {pc}
1:  movs r1, r0
    movs r0, #5
    bx   lr
    hook HOOK_HALF, 0xA003F30A, 0xB500, 0x0001
    hook HOOK_BL, 0xA003F30C, iso50_clamp_2, 0xB5002005

// Auto-ISO lower limit 50 (Sv 4.0 = 0x40000):
// - 0xA000D5AC: clamp initial Auto-ISO Sv to [min_sv, max_sv] around 4.0 (identical to stock for min >= ISO 100)
// - 0xA000D5CE: sv_to_iso_idx scans from index 1 (ISO 50) instead of 2 (ISO 100)
// - 0xA000D7C0: ae_commit_solved_iso map table[1] = 1 (stock has 2, clamping ISO 50 -> 100 in 0xA0939A51)
// - up-loops (A/S/P1/P2): `adds r4,r4,r6; adds r5,r5,r6`; jump 4.0 -> 5.0 by 1.0 EV so 1/3-EV mode never lands on 64/80
fn autoiso_sv_step
    movs r2, #4
    lsls r2, r2, #16
    movs r3, r6
    cmp  r5, r2
    bne  1f
    lsrs r3, r2, #2
1:  adds r4, r4, r3
    adds r5, r5, r3
    bx   lr
    hook HOOK_HALF, 0xA000D5B6, 0x2104, 0x2105
    hook HOOK_HALF, 0xA000D5D4, 0x2001, 0x2002
    hook HOOK_HALF, 0xA000D7C0, 0x0501, 0x0502
    hook HOOK_BL, 0xA000D844, autoiso_sv_step, 0x19A419AD
    hook HOOK_BL, 0xA000D8BA, autoiso_sv_step, 0x19A419AD
    hook HOOK_BL, 0xA000D9AA, autoiso_sv_step, 0x19A419AD
    hook HOOK_BL, 0xA000D9F0, autoiso_sv_step, 0x19A419AD

// UI string lookup 0xA0044AAC after `push {r4-r6,lr}` (SP 8B-aligned): r0 sid, r1 lang
fn str_hook
    push {r0, r1, r2, lr}
    bl   dp2m_str
    cmp  r0, #0
    bne  1f
    pop  {r0, r1, r2, r3}
    mov  lr, r3
    movs r5, r0
    movs r4, r5
    bx   lr
1:  add  sp, #16
    pop  {r4, r5, r6, pc}
    hook HOOK_BL, 0xA0044AAE, str_hook, 0x0005002C

// Get_Menu_Table 0xA004CD08 after `push {r4,lr}`; the miss path leaves `cmp r0,#0x6b` flags for the stock `bgt`
fn menu_hook
    push {r0, r1, r2, lr}
    bl   menu_lookup
    cmp  r0, #0
    beq  1f
    add  sp, #16
    pop  {r4, pc}
1:  pop  {r0, r1, r2, r3}
    mov  lr, r3
    cmp  r0, #0x6B
    beq  2f
    bx   lr
2:  ldr  r3, =stock_get_menu_6b
    bx   r3
    .ltorg
    hook HOOK_BL, 0xA004CD0A, menu_hook, 0x286BD03E

// capture IPC cmd 0x20 @ 0xA00360A2: r4 cmd, meta at caller sp+0x14, live r1 = 1.0f
fn ipc_hook
    push {r1, r2, r3, lr}
    ldr  r1, [sp, #0x24]
    movs r0, r4
    bl   ipc_pack
    movs r0, #0
    pop  {r1, r2, r3, pc}
    hook HOOK_BL, 0xA00360A2, ipc_hook, 0x20007160
    hook HOOK_HALF, 0xA00360A6, 0x46C0, 0x71A0

// maker note color mode switch 0xA0148CF8 (`blx __ARM_common_switch8`, r3 = cm, table at lr): films skip the
// stock switch, whose default writes "Standard", and rejoin after its common tail memcpy
fn mn_cm_hook
    cmp  r3, #9
    bhs  1f
    ldr  r2, =0xA0173358
    bx   r2
1:  movs r0, r3
    ldr  r1, [sp, #0x44]
    ldr  r2, [sp, #0x40]
    bl   mn_film
    ldr  r0, =0xA0148D2F
    bx   r0
    .ltorg
    hook HOOK_BL, 0xA0148CF8, mn_cm_hook, 0xF02AEB2E

// playback maker note parsers, at the store all color mode cases share; 0 = no stock name matched
.macro pb_cm name, site, str, rec, ld, expect
fn \name
    cmp  r0, #0
    bne  1f
    push {r1, lr}
    movs r0, \str
    bl   pb_film_cm
    pop  {r1, r2}
    mov  lr, r2
1:  strb r0, [\rec, #0x17]
    ldr  r0, [sp, #\ld]
    bx   lr
    hook HOOK_BL, \site, \name, \expect
.endm
    pb_cm pb_cm_jpg, 0xA0147938, r7, r4, 0x48, 0x75E09812
    pb_cm pb_cm_x3f, 0xA0132E9A, r6, r5, 0xE4, 0x75E89839

// LV YCC capture 0xA0033F1A after `push {r4-r7,lr}`: replaces `ldr r5,=lit; movs r7,r0`
fn lv_ycap_hook
    movs r7, r0
    ldr  r3, =lv_planes
    ldr  r2, [r0, #0]
    str  r2, [r3, #0]
    ldr  r2, [r0, #4]
    str  r2, [r3, #4]
    ldr  r2, [r0, #8]
    str  r2, [r3, #8]
    ldr  r2, [r3, #12]
    adds r2, #1
    str  r2, [r3, #12]
    ldr  r5, =0xA07B06F8
    bx   lr
    .ltorg
    hook HOOK_BL, 0xA0033F1C, lv_ycap_hook, 0x4D730007

// Get_Bkt_EV_Q16 0xA0002FE4 after `push {r4-r6,lr}`; orders 4/6/7 are 5-shot:
//   4: [0,+2,+1,-1,-2]  6: [+2,+1,0,-1,-2]  7: [-2,-1,0,+1,+2]  (x step q16)
fn bkt5_hook
    movs r5, r0
    mov  r6, lr
    bl   stock_get_bkt_ord
    cmp  r0, #4
    bhs  1f
    movs r4, #0
    bx   r6
1:  ldr  r1, =0xA07FBE5A
    ldrh r1, [r1]
    lsls r1, r1, #2
    ldr  r2, =0xA0176F14
    ldr  r2, [r2, r1]
    movs r3, #0
    cmp  r5, #4
    bhi  5f
    cmp  r0, #6
    beq  3f
    cmp  r0, #7
    beq  4f
    cmp  r0, #4
    bne  5f
    cmp  r5, #0
    beq  5f
    movs r3, #3
    cmp  r5, #2
    ble  2f
    movs r3, #2
2:  subs r3, r3, r5
    b    6f
3:  movs r3, #2
    subs r3, r3, r5
    b    6f
4:  subs r3, r5, #2
6:  muls r3, r2
5:  movs r0, r3
    pop  {r4, r5, r6, pc}
    .ltorg
    hook HOOK_BL, 0xA0002FE6, bkt5_hook, 0x00052400

// auto-ISO min shutter: A mode 0xA000D83E (`ldr r0,=0x5AAAA; b +8`), P mode 0xA000D9A2/E8 (`movs r0,#9; lsls r0,#16`)
fn autoiso_a_hook
    ldr  r2, =0xA093968B
    ldrb r2, [r2]
    cmp  r2, #6
    bls  1f
    movs r2, #0
1:  lsls r2, r2, #2
    ldr  r3, =dp2m_tv_min_q16
    ldr  r0, [r3, r2]
    mov  r2, lr
    adds r2, #10
    bx   r2
    .ltorg
    hook HOOK_BL, 0xA000D83E, autoiso_a_hook, 0x48EFE004

fn autoiso_p_hook
    ldr  r2, =0xA093968B
    ldrb r2, [r2]
    subs r2, #1
    cmp  r2, #5
    bhi  1f
    lsls r2, r2, #2
    ldr  r3, =dp2m_tv_min_q16 + 4
    ldr  r0, [r3, r2]
    movs r2, #3
    lsls r2, r2, #16
    adds r0, r0, r2
    bx   lr
1:  movs r0, #9
    lsls r0, r0, #16
    bx   lr
    .ltorg
    hook HOOK_BL, 0xA000D9A2, autoiso_p_hook, 0x20090400
    hook HOOK_BL, 0xA000D9E8, autoiso_p_hook, 0x20090400

// icon lookup 0xA0042AAC after `push {r4-r6,lr}`: ids >= 0x400 are dp2m label icons
fn icon_hook
    lsrs r1, r0, #10
    bne  1f
    movs r4, r0
    movs r2, #0xFF
    bx   lr
1:  bl   label_icon
    pop  {r4, r5, r6, pc}
    hook HOOK_BL, 0xA0042AAE, icon_hook, 0x000422FF

// QS strip 0xA0043E9C (`ldr r1,=iso getter; ldr r0,[r0,#0x14]`): color mode joins ISO on the scrolling strip
fn qs_strip_hook
    ldr  r0, [r0, #0x14]
    ldr  r1, =stock_get_cm
    cmp  r0, r1
    beq  1f
    ldr  r1, =stock_get_iso
1:  bx   lr
    .ltorg
    hook HOOK_BL, 0xA0043E9C, qs_strip_hook, 0x49F36940

// QS scroll call 0xA0043EAA: ISO and Colour Mode share QS_STATE+6, but only the ISO
// setter resets it. Clamp a foreign scroll before the stock 8-tile loop indexes rows.
fn qs_scroll_clamp
    cmp  r1, #8
    bhs  1f
    movs r2, #0
    b    2f
1:  movs r3, r1
    subs r3, #8
    cmp  r2, r3
    bls  2f
    movs r2, r3
2:  ldr  r3, =0xA0043CC7
    bx   r3
    .ltorg
    hook HOOK_BL, 0xA0043EAA, qs_scroll_clamp, 0xF7FFFF0C

// QS strip selection call 0xA0043D06 (`bl 0xA004D4F0`): stock 0xA004D4F0 returns row count n
// on no-match; clamp sel >= n (signed, keeping -1) to n-1 so the 8-tile loop stays in bounds
// without altering 0xA004D4F0 for other callers (e.g. QS slot icons at 0xA00439C4).
fn qs_sel_clamp
    push {r1, r2, r3, r4, r5, lr}
    movs r4, r0
    ldr  r3, =0xA004D4D7
    blx  r3
    movs r5, r0
    movs r0, r4
    ldr  r3, =0xA004D4F1
    blx  r3
    cmp  r0, r5
    blt  1f
    subs r0, r5, #1
1:  pop  {r1, r2, r3, r4, r5, pc}
    .ltorg
    hook HOOK_BL, 0xA0043D06, qs_sel_clamp, 0xF009FBF3

// Boot MPU init (0xA0172170 / 0xA0172190):
// - 0xA0172170: `mov r0, #0xC` enables CP15 c2 I-cache on Region 2 (SDRAM) + Region 3 (NOR)
// - 0xA0172190: `orr r0, r0, #0x600` (ARM imm 0x06 ror 24 = 0xE3800C06) sets Region 2 I-AP = 0b0110 (Priv/User exec)
    hook HOOK_WORD, 0xA0172170, 0xE3A0000C, 0xE3A00008
    hook HOOK_WORD, 0xA0172190, 0xE3800C06, 0xE3800000

// Shutter scan 0xA01690B6 (`bl 0xA00363AE`): report busy (1) while SCREEN_DOOM (0x21) is active
// so half/full shutter acts as a pure game fire button without S1 LV unwind or S2 capture.
fn shutter_guard_hook
    push {r4, lr}
    bl   stock_ui_screen_id
    cmp  r0, #0x21
    bne  1f
    movs r0, #1
    pop  {r4, pc}
1:  bl   stock_busy_check
    pop  {r4, pc}
    hook HOOK_CALL, 0xA01690B6, shutter_guard_hook, 0xA00363AE
