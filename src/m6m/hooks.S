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

// iso clamps 0xA003F2CC / 0xA003F30A: `movs r1,r0; push {lr}; movs r0,#5` -> `push {lr}; bl`; ISO 50 (1) passes through
fn iso50_clamp_1
    cmp  r0, #1
    bne  1f
    pop  {pc}
1:  movs r1, r0
    movs r0, #5
    bx   lr
    hook HOOK_HALF, 0xA003F2CC, 0xB500, 0x0001
    hook HOOK_BL, 0xA003F2CE, iso50_clamp_1, 0xB5002005

fn iso50_clamp_2
    cmp  r0, #1
    bne  1f
    movs r0, #2
    pop  {pc}
1:  movs r1, r0
    movs r0, #5
    bx   lr
    hook HOOK_HALF, 0xA003F30A, 0xB500, 0x0001
    hook HOOK_BL, 0xA003F30C, iso50_clamp_2, 0xB5002005

// UI string lookup 0xA0044AAC after `push {r4-r6,lr}`: r0 sid, r1 lang
fn str_hook
    push {r0, r1, lr}
    bl   dp2m_str
    cmp  r0, #0
    bne  1f
    pop  {r0, r1, r2}
    mov  lr, r2
    movs r5, r0
    movs r4, r5
    bx   lr
1:  add  sp, #12
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
    bge  1f
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
