// ipc cmd 0x20 packer (0xA00360A2) & lv plane ptr capture (0xA0033F1C)
#include "m6m_addrs.h"

    .syntax unified
    .thumb

    .section .text.lv_ycap_hook, "ax", %progbits
    movs r7, r0
    ldr  r3, =VA_RAM_LIVE_Y_PTR
    ldr  r2, [r0, #0]
    str  r2, [r3, #0]
    ldr  r2, [r0, #4]
    str  r2, [r3, #4]
    ldr  r2, [r0, #8]
    str  r2, [r3, #8]
    ldr  r2, =VA_RAM_LV_FCNT
    ldr  r3, [r2]
    adds r3, r3, #1
    str  r3, [r2]
    ldr  r5, =SYM_LV_YCAP_LIT
    bx   lr
    .ltorg

// stock clamps meta[+0x1E] to 0..7 & [+0x1D] to 0..1; pack full mode into cmd[5] & (fill<<5)|(fx<<2)|cs into cmd[6], preserving r1 (1.0f)
    .section .text.ipc_cmd_hook, "ax", %progbits
    push {r1, r2, r3, lr}
    ldr  r2, [sp, #0x24]
    ldrb r0, [r2, #0x1E]
    cmp  r0, #23
    blt  cm_ok
    ldr  r1, =VA_NVRAM_PAPER
    ldrb r1, [r1]
    cmp  r0, #38
    bgt  chk_bw
    movs r3, r0
    subs r3, #23
    cmp  r1, #0
    bne  neg_has_p
    ldr  r2, =VA_AUTO_PAPER_TBL
    ldrb r1, [r2, r3]
    b    neg_pk
neg_has_p:
    subs r1, #1
neg_pk:
    lsls r3, r3, #3
    adds r0, r3, r1
    adds r0, #23
    b    cm_ok
chk_bw:
    cmp  r0, #43
    bgt  chk_sd
    cmp  r1, #0
    beq  bw_p0
    subs r1, #1
bw_p0:
    movs r3, r0
    subs r3, #39
    lsls r3, r3, #3
    adds r0, r3, r1
    adds r0, #151
    b    cm_ok
chk_sd:
    cmp  r0, #44
    bne  cm_ok
    cmp  r1, #0
    beq  sd_p0
    subs r1, #1
sd_p0:
    movs r0, #191
    adds r0, r0, r1
cm_ok:
    strb r0, [r4, #5]
    ldr  r2, [sp, #0x24]
    ldrb r0, [r2, #0x1D]
    movs r3, #3
    ands r0, r3
    ldr  r1, =VA_NVRAM_FX
    ldrb r1, [r1]
    movs r3, #7
    ands r1, r3
    lsls r1, r1, #2
    orrs r0, r1
    ldr  r1, =VA_NVRAM_FILL
    ldrb r1, [r1]
    movs r3, #3
    ands r1, r3
    lsls r1, r1, #5
    orrs r0, r1
    strb r0, [r4, #6]
    pop  {r1, r2, r3, pc}
    .ltorg
