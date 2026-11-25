// ui strings (sid 672..754), menu table dispatcher, osd icon map, nvram getters/setters
#include "m6m_addrs.h"

    .syntax unified
    .thumb

    .section .text.str_hook, "ax", %progbits
    movs r5, r0
    movs r4, r5
    ldr  r2, =672
    cmp  r0, r2
    blt  1f
    subs r2, r0, r2
    cmp  r2, #83
    bge  1f
    lsls r2, r2, #2
    ldr  r0, =VA_STR_PTRS_83
    ldr  r0, [r0, r2]
    pop  {r4, r5, r6, pc}
1:  bx   lr
    .ltorg

    .section .text.osd_icon_hook, "ax", %progbits
    push {lr}
    ldr  r3, =(SYM_GET_CM | 1)
    blx  r3
    cmp  r0, #9
    blt  3f
    cmp  r0, #14
    blt  1f
    cmp  r0, #18
    ble  2f
    cmp  r0, #39
    blt  1f
    cmp  r0, #43
    ble  2f
1:  movs r0, #4
    pop  {pc}
2:  movs r0, #2
3:  pop  {pc}
    .ltorg

    .section .text.get_menu_hook, "ax", %progbits
    cmp  r0, #0x09
    beq  10f
    cmp  r0, #0x37
    beq  11f
    cmp  r0, #0x10
    beq  12f
    cmp  r0, #0x14
    beq  13f
    cmp  r0, #0x17
    beq  14f
    cmp  r0, #0x28
    beq  15f
    cmp  r0, #0x3E
    beq  16f
    cmp  r0, #0xF0
    beq  17f
    cmp  r0, #0xF1
    beq  18f
    cmp  r0, #0xF2
    beq  19f
    cmp  r0, #0xF3
    beq  20f
    cmp  r0, #0xF4
    beq  21f
    cmp  r0, #0x6B
    beq  9f
    bx   lr
9:  ldr  r3, =(SYM_GET_MENU_6B | 1)
    bx   r3
10: ldr  r0, =VA_COLOR_MENU_44
    pop  {r4, pc}
11: ldr  r0, =VA_GRID_MENU_17
    pop  {r4, pc}
12: ldr  r0, =VA_SHOOT_P3_NEW
    pop  {r4, pc}
13: ldr  r0, =VA_SETUP_P2_V5
    pop  {r4, pc}
14: ldr  r0, =VA_SETUP_P5_NEW
    pop  {r4, pc}
15: ldr  r0, =VA_BKTORD_MENU_6
    pop  {r4, pc}
16: ldr  r0, =VA_ISO_AUTO_3
    pop  {r4, pc}
17: ldr  r0, =VA_PAPER_MENU
    pop  {r4, pc}
18: ldr  r0, =VA_FILMFX_MENU
    pop  {r4, pc}
19: ldr  r0, =VA_X3FILL_MENU
    pop  {r4, pc}
20: ldr  r0, =VA_MINSHUTTER_MENU
    pop  {r4, pc}
21: ldr  r0, =VA_PEAKCOL_MENU
    pop  {r4, pc}
    .ltorg

    .section .text.get_paper, "ax", %progbits
    ldr  r0, =VA_NVRAM_PAPER
    ldrb r0, [r0]
    bx   lr
    .ltorg

    .section .text.set_paper, "ax", %progbits
    push {lr}
    ldr  r1, =VA_NVRAM_PAPER
    strb r0, [r1]
    ldr  r3, =(SYM_LV_REFRESH | 1)
    blx  r3
    pop  {pc}
    .ltorg

    .section .text.get_fx, "ax", %progbits
    ldr  r0, =VA_NVRAM_FX
    ldrb r0, [r0]
    bx   lr
    .ltorg

    .section .text.set_fx, "ax", %progbits
    ldr  r1, =VA_NVRAM_FX
    strb r0, [r1]
    bx   lr
    .ltorg

    .section .text.get_fill, "ax", %progbits
    ldr  r0, =VA_NVRAM_FILL
    ldrb r0, [r0]
    bx   lr
    .ltorg

    .section .text.set_fill, "ax", %progbits
    ldr  r1, =VA_NVRAM_FILL
    strb r0, [r1]
    bx   lr
    .ltorg

    .section .text.get_minsh, "ax", %progbits
    ldr  r0, =VA_NVRAM_MINSHUTTER
    ldrb r0, [r0]
    bx   lr
    .ltorg

    .section .text.set_minsh, "ax", %progbits
    ldr  r1, =VA_NVRAM_MINSHUTTER
    strb r0, [r1]
    bx   lr
    .ltorg

    .section .text.get_peakc, "ax", %progbits
    ldr  r0, =VA_NVRAM_PEAKCOL
    ldrb r0, [r0]
    bx   lr
    .ltorg

    .section .text.set_peakc, "ax", %progbits
    ldr  r1, =VA_NVRAM_PEAKCOL
    strb r0, [r1]
    bx   lr
    .ltorg
