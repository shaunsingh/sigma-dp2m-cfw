// lv mono, cmcm, gamma, & grid draw trampolines
#include "m6m_addrs.h"

    .syntax unified
    .thumb

    .section .text.lv_mono_hook, "ax", %progbits
    cmp  r1, #14
    blt  1f
    cmp  r1, #18
    ble  2f
    cmp  r1, #39
    blt  1f
    cmp  r1, #43
    bgt  1f
2:  movs r1, #2
1:  ldr  r3, =(SYM_LV_MONO_FN | 1)
    bx   r3
    .ltorg

    .section .text.lv_cmcm_tramp, "ax", %progbits
    cmp  r1, #9
    blt  1f
    cmp  r1, #43
    bgt  1f
    ldr  r3, =(VA_LV_CMCM_C | 1)
    bx   r3
1:  ldr  r3, =(SYM_LV_CMCM_FN | 1)
    bx   r3
    .ltorg

    .section .text.lv_gamma_wrap, "ax", %progbits
    mov  r3, sp
    ldr  r0, =(VA_LV_GAMMA_C | 1)
    mov  r12, r0
    movs r0, #0
    bx   r12
    .ltorg

    .section .text.grid_draw_tramp, "ax", %progbits
    push {r0, lr}
    ldr  r3, =(SYM_GET_GRID | 1)
    blx  r3
    cmp  r0, #7
    blt  1f
    cmp  r0, #16
    bgt  1f
    pop  {r0}
    ldr  r3, =(VA_GRID_DRAW_C | 1)
    blx  r3
    pop  {pc}
1:  pop  {r0}
    ldr  r3, =(SYM_GRID_DRAW_FN | 1)
    blx  r3
    pop  {pc}
    .ltorg
