// iso 50 pass-through hooks (0xA003F2CC/30A) & service-screen stub
#include "m6m_addrs.h"

    .syntax unified
    .thumb

    .section .text.safe_ret0, "ax", %progbits
    movs r0, #0
    bx   lr

    .section .text.iso50_hook_1, "ax", %progbits
    cmp  r0, #1
    bne  1f
    movs r0, #1
    pop  {pc}
1:  movs r1, r0
    movs r0, #5
    bx   lr

    .section .text.iso50_hook_2, "ax", %progbits
    cmp  r0, #1
    bne  1f
    movs r0, #2
    pop  {pc}
1:  movs r1, r0
    movs r0, #5
    bx   lr
