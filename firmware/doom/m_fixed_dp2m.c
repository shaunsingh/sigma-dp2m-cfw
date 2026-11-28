/* SPDX-License-Identifier: GPL-2.0-or-later */
/* ARMv5TE FixedMul (SMULL) & Bit-Exact FixedDiv (48/32 shift-subtract, zero ldivmod) */
#include <stdlib.h>
#include "doomtype.h"
#include "i_system.h"
#include "m_fixed.h"

fixed_t FixedMul(fixed_t a, fixed_t b) {
    int32_t lo, hi;
    __asm__(
        "smull %0, %1, %2, %3\n\t"
        "mov   %0, %0, lsr #16\n\t"
        "orr   %0, %0, %1, lsl #16"
        : "=&r"(lo), "=&r"(hi)
        : "r"(a), "r"(b)
    );
    return lo;
}

fixed_t FixedDiv(fixed_t a, fixed_t b) {
    uint32_t ua = (a < 0) ? (0u - (uint32_t)a) : (uint32_t)a;
    uint32_t ub = (b < 0) ? (0u - (uint32_t)b) : (uint32_t)b;
    if ((ua >> 14) >= ub) {
        return ((a ^ b) < 0) ? INT_MIN : INT_MAX;
    }
    /* Compute exact ((uint64_t)ua << 16) / ub in 31 shift-subtract steps (quotient < 2^30) */
    uint32_t rem = ua >> 15;
    uint32_t lo  = ua << 17;
    uint32_t q   = 0u;
    for (int i = 0; i < 31; i++) {
        rem = (rem << 1) | (lo >> 31);
        lo <<= 1;
        q <<= 1;
        if (rem >= ub) {
            rem -= ub;
            q |= 1u;
        }
    }
    return ((a ^ b) < 0) ? -(fixed_t)q : (fixed_t)q;
}
