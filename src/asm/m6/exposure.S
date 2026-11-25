// 5-shot bkt q16 apex ev hook (0xA0002FE6) & auto-iso min shutter hooks
#include "m6m_addrs.h"

    .syntax unified
    .thumb

// ord<4 returns to stock via r6; 5-shot apex-sign offsets * step_q16:
//   4: [0,+2,+1,-1,-2]  6: [+2,+1,0,-1,-2]  7: [-2,-1,0,+1,+2]
    .section .text.bkt5_hook, "ax", %progbits
    movs r5, r0
    mov  r6, lr
    ldr  r3, =(SYM_GET_BKT_ORD | 1)
    blx  r3
    cmp  r0, #4
    bge  do_5shot
    movs r4, #0
    bx   r6
do_5shot:
    ldr  r1, =SYM_BKT_STEP_IDX
    ldrh r1, [r1]
    lsls r1, r1, #2
    ldr  r2, =SYM_BKT_STEP_TBL
    ldr  r2, [r2, r1]
    movs r3, #0
    cmp  r5, #4
    bhi  done_5
    cmp  r0, #6
    beq  ord_6
    cmp  r0, #7
    beq  ord_7
    cmp  r0, #4
    bne  done_5
    cmp  r5, #0
    beq  done_5
    cmp  r5, #2
    bgt  o4_hi
    movs r3, #3
    subs r3, r3, r5
    b    mul_5
o4_hi:
    movs r3, #2
    subs r3, r3, r5
    b    mul_5
ord_6:
    movs r3, #2
    subs r3, r3, r5
    b    mul_5
ord_7:
    subs r3, r5, #2
mul_5:
    muls r2, r3
    movs r3, r2
done_5:
    movs r0, r3
    pop  {r4, r5, r6, pc}
    .ltorg

    .section .text.autoiso_a_hook, "ax", %progbits
    ldr  r2, =VA_NVRAM_MINSHUTTER
    ldrb r2, [r2]
    cmp  r2, #6
    bls  1f
    movs r2, #0
1:  lsls r2, r2, #2
    ldr  r3, =VA_TV_MIN_TBL
    ldr  r0, [r3, r2]
    mov  r2, lr
    adds r2, #10
    bx   r2
    .ltorg

    .section .text.autoiso_p_hook, "ax", %progbits
    ldr  r2, =VA_NVRAM_MINSHUTTER
    ldrb r2, [r2]
    cmp  r2, #0
    beq  1f
    cmp  r2, #6
    bhi  1f
    lsls r2, r2, #2
    ldr  r3, =VA_TV_MIN_TBL
    ldr  r0, [r3, r2]
    movs r2, #3
    lsls r2, r2, #16
    adds r0, r0, r2
    bx   lr
1:  movs r0, #9
    lsls r0, r0, #16
    bx   lr
    .ltorg
