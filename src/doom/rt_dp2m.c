/* SPDX-License-Identifier: GPL-2.0-or-later */
/* Freestanding ARMv5TE Big-Endian Runtime: setjmp/longjmp, mem*, CLZ div & soft-float */
#include <stddef.h>
#include <stdint.h>
#include "dp2m_doom.h"

__attribute__((naked))
int dp2m_setjmp(dp2m_jmp_buf env __attribute__((unused))) {
    __asm__ volatile(
        "stmia r0, {r4-r11, sp, lr}\n\t"
        "mov   r0, #0\n\t"
        "bx    lr\n\t"
    );
}

__attribute__((naked, noreturn))
void dp2m_longjmp(dp2m_jmp_buf env __attribute__((unused)),
                  int val __attribute__((unused))) {
    __asm__ volatile(
        "ldmia r0, {r4-r11, sp, lr}\n\t"
        "movs  r0, r1\n\t"
        "moveq r0, #1\n\t"
        "bx    lr\n\t"
    );
}

__attribute__((noinline))
void *memset(void *d, int c, size_t n) {
    uint8_t *p = (uint8_t *)d;
    uint8_t b = (uint8_t)c;
    if (n >= 8) {
        while ((uintptr_t)p & 3u) { *p++ = b; n--; }
        uint32_t v = (uint32_t)b * 0x01010101u;
        uint32_t *w = (uint32_t *)(void *)p;
        while (n >= 16) {
            w[0] = v; w[1] = v; w[2] = v; w[3] = v;
            w += 4;
            n -= 16;
        }
        while (n >= 4) { *w++ = v; n -= 4; }
        p = (uint8_t *)w;
    }
    while (n--) *p++ = b;
    return d;
}

__attribute__((noinline))
void *memcpy(void *d, const void *s, size_t n) {
    uint8_t *p = (uint8_t *)d;
    const uint8_t *q = (const uint8_t *)s;
    if (n >= 8) {
        while ((uintptr_t)p & 3u) { *p++ = *q++; n--; }
        uint32_t *wp = (uint32_t *)(void *)p;
        uint32_t sh = ((uintptr_t)q & 3u) << 3;
        if (!sh) {
            const uint32_t *wq = (const uint32_t *)(const void *)q;
            while (n >= 16) {
                uint32_t a = wq[0], b = wq[1], c = wq[2], e = wq[3];
                wp[0] = a; wp[1] = b; wp[2] = c; wp[3] = e;
                wq += 4;
                wp += 4;
                n -= 16;
            }
            while (n >= 4) { *wp++ = *wq++; n -= 4; }
            q = (const uint8_t *)wq;
        } else {
            const uint32_t *wq = (const uint32_t *)(const void *)((uintptr_t)q & ~3u);
            uint32_t rsh = 32u - sh, w0 = *wq++;
            while (n >= 4) {
                uint32_t w1 = *wq++;
                *wp++ = (w0 << sh) | (w1 >> rsh);
                w0 = w1;
                n -= 4;
            }
            q = (const uint8_t *)wq - (4u - (sh >> 3));
        }
        p = (uint8_t *)wp;
    }
    while (n--) *p++ = *q++;
    return d;
}

__attribute__((noinline))
void *memmove(void *d, const void *s, size_t n) {
    uint8_t *p = (uint8_t *)d;
    const uint8_t *q = (const uint8_t *)s;
    if (p == q || n == 0) return d;
    if (p < q || (size_t)(p - q) >= n) return memcpy(d, s, n);
    p += n;
    q += n;
    while (n--) *--p = *--q;
    return d;
}

int memcmp(const void *a, const void *b, size_t n) {
    const uint8_t *pa = (const uint8_t *)a;
    const uint8_t *pb = (const uint8_t *)b;
    while (n--) {
        if (*pa != *pb) return (int)*pa - (int)*pb;
        pa++;
        pb++;
    }
    return 0;
}

void *memchr(const void *s, int c, size_t n) {
    const uint8_t *p = (const uint8_t *)s;
    uint8_t b = (uint8_t)c;
    while (n--) {
        if (*p == b) return (void *)(uintptr_t)p;
        p++;
    }
    return NULL;
}

void __aeabi_memset(void *d, size_t n, int c) { memset(d, c, n); }
void __aeabi_memset4(void *d, size_t n, int c) __attribute__((alias("__aeabi_memset")));
void __aeabi_memset8(void *d, size_t n, int c) __attribute__((alias("__aeabi_memset")));
void __aeabi_memclr(void *d, size_t n) { memset(d, 0, n); }
void __aeabi_memclr4(void *d, size_t n) __attribute__((alias("__aeabi_memclr")));
void __aeabi_memclr8(void *d, size_t n) __attribute__((alias("__aeabi_memclr")));
void __aeabi_memcpy(void *d, const void *s, size_t n) { memcpy(d, s, n); }
void __aeabi_memcpy4(void *d, const void *s, size_t n) __attribute__((alias("__aeabi_memcpy")));
void __aeabi_memcpy8(void *d, const void *s, size_t n) __attribute__((alias("__aeabi_memcpy")));
void __aeabi_memmove(void *d, const void *s, size_t n) { memmove(d, s, n); }
void __aeabi_memmove4(void *d, const void *s, size_t n) __attribute__((alias("__aeabi_memmove")));
void __aeabi_memmove8(void *d, const void *s, size_t n) __attribute__((alias("__aeabi_memmove")));

/* Fast ARMv5 CLZ shift-subtract unsigned 32-bit division */
static inline uint32_t udiv32_rem(uint32_t num, uint32_t den, uint32_t *rem_out) {
    if (den == 0u) {
        if (rem_out) *rem_out = 0u;
        return 0u;
    }
    if (num < den) {
        if (rem_out) *rem_out = num;
        return 0u;
    }
    int shift = __builtin_clz(den) - __builtin_clz(num);
    uint32_t d = den << shift;
    uint32_t bit = 1u << shift;
    uint32_t q = 0u;
    while (bit != 0u) {
        if (num >= d) {
            num -= d;
            q |= bit;
        }
        d >>= 1;
        bit >>= 1;
    }
    if (rem_out) *rem_out = num;
    return q;
}

uint32_t __aeabi_uidiv(uint32_t num, uint32_t den) {
    return udiv32_rem(num, den, NULL);
}

int32_t __aeabi_idiv(int32_t num, int32_t den) {
    if (den == 0) return 0;
    uint32_t neg = (uint32_t)(num ^ den) >> 31;
    uint32_t un = (num < 0) ? (0u - (uint32_t)num) : (uint32_t)num;
    uint32_t ud = (den < 0) ? (0u - (uint32_t)den) : (uint32_t)den;
    uint32_t q = udiv32_rem(un, ud, NULL);
    return neg ? -(int32_t)q : (int32_t)q;
}

/* AEABI value_in_regs: r0 = quotient, r1 = remainder */
__attribute__((naked))
void __aeabi_uidivmod(void) {
    __asm__ volatile(
        "push  {r4, lr}\n\t"
        "sub   sp, sp, #8\n\t"
        "mov   r2, sp\n\t"
        "bl    __dp2m_uidivmod_c\n\t"
        "ldr   r1, [sp]\n\t"
        "add   sp, sp, #8\n\t"
        "pop   {r4, pc}\n\t"
    );
}

uint32_t __dp2m_uidivmod_c(uint32_t num, uint32_t den, uint32_t *rem) {
    return udiv32_rem(num, den, rem);
}

__attribute__((naked))
void __aeabi_idivmod(void) {
    __asm__ volatile(
        "push  {r4, lr}\n\t"
        "sub   sp, sp, #8\n\t"
        "mov   r2, sp\n\t"
        "bl    __dp2m_idivmod_c\n\t"
        "ldr   r1, [sp]\n\t"
        "add   sp, sp, #8\n\t"
        "pop   {r4, pc}\n\t"
    );
}

int32_t __dp2m_idivmod_c(int32_t num, int32_t den, int32_t *rem) {
    if (den == 0) { *rem = 0; return 0; }
    uint32_t un = (num < 0) ? (0u - (uint32_t)num) : (uint32_t)num;
    uint32_t ud = (den < 0) ? (0u - (uint32_t)den) : (uint32_t)den;
    uint32_t r = 0u;
    uint32_t q = udiv32_rem(un, ud, &r);
    *rem = (num < 0) ? -(int32_t)r : (int32_t)r;
    return ((num ^ den) < 0) ? -(int32_t)q : (int32_t)q;
}

/* Minimal soft-float shims for cold demo/mouse config paths in doomgeneric */
uint32_t __aeabi_i2f(int32_t x) {
    if (x == 0) return 0u;
    uint32_t sign = (x < 0) ? 0x80000000u : 0u;
    uint32_t ux = (x < 0) ? (0u - (uint32_t)x) : (uint32_t)x;
    int lz = __builtin_clz(ux);
    uint32_t exp = (uint32_t)(158 - lz);
    uint32_t mant = (lz >= 8) ? (ux << (lz - 8)) : (ux >> (8 - lz));
    return sign | (exp << 23) | (mant & 0x007FFFFFu);
}

uint32_t __aeabi_fmul(uint32_t a, uint32_t b) {
    uint32_t sa = a & 0x7FFFFFFFu, sb = b & 0x7FFFFFFFu;
    if (sa == 0u || sb == 0u) return (a ^ b) & 0x80000000u;
    int32_t ea = (int32_t)((sa >> 23) & 0xFFu), eb = (int32_t)((sb >> 23) & 0xFFu);
    uint64_t ma = (sa & 0x7FFFFFu) | 0x800000u;
    uint64_t mb = (sb & 0x7FFFFFu) | 0x800000u;
    uint64_t prod = (ma * mb) >> 23;
    int32_t exp = ea + eb - 127;
    if (prod & 0x1000000ull) { prod >>= 1; exp++; }
    if (exp <= 0) return (a ^ b) & 0x80000000u;
    if (exp >= 255) return ((a ^ b) & 0x80000000u) | 0x7F800000u;
    return ((a ^ b) & 0x80000000u) | ((uint32_t)exp << 23) | ((uint32_t)prod & 0x7FFFFFu);
}

uint32_t __aeabi_fdiv(uint32_t a, uint32_t b) {
    uint32_t sa = a & 0x7FFFFFFFu, sb = b & 0x7FFFFFFFu;
    uint32_t sign = (a ^ b) & 0x80000000u;
    if (sb == 0u) return sign | 0x7F800000u;
    if (sa == 0u) return sign;
    int32_t ea = (int32_t)((sa >> 23) & 0xFFu), eb = (int32_t)((sb >> 23) & 0xFFu);
    uint32_t num = (sa & 0x7FFFFFu) | 0x800000u;
    uint32_t den = (sb & 0x7FFFFFu) | 0x800000u;
    int32_t exp = ea - eb + 127;
    if (num < den) { num <<= 1; exp--; }
    uint32_t q = 0u;
    for (int i = 23; i >= 0; i--) {
        if (num >= den) { num -= den; q |= (1u << i); }
        num <<= 1;
    }
    if (exp <= 0) return sign;
    if (exp >= 255) return sign | 0x7F800000u;
    return sign | ((uint32_t)exp << 23) | (q & 0x7FFFFFu);
}

uint32_t __aeabi_fadd(uint32_t a, uint32_t b);
uint32_t __aeabi_fsub(uint32_t a, uint32_t b) {
    return __aeabi_fadd(a, b ^ 0x80000000u);
}

uint32_t __aeabi_fadd(uint32_t a, uint32_t b) {
    uint32_t ua = a & 0x7FFFFFFFu, ub = b & 0x7FFFFFFFu;
    if (ua < ub) { uint32_t t = a; a = b; b = t; ua = a & 0x7FFFFFFFu; ub = b & 0x7FFFFFFFu; }
    if (ub == 0u) return a;
    uint32_t sign = a & 0x80000000u;
    int32_t ea = (int32_t)(ua >> 23), eb = (int32_t)(ub >> 23);
    int32_t diff = ea - eb;
    if (diff > 24) return a;
    uint32_t ma = ((ua & 0x7FFFFFu) | 0x800000u) << 3;
    uint32_t mb = (((ub & 0x7FFFFFu) | 0x800000u) << 3) >> diff;
    uint32_t m = ((a ^ b) & 0x80000000u) ? (ma - mb) : (ma + mb);
    if (m == 0u) return 0u;
    while (m >= (1u << 27)) { m >>= 1; ea++; }
    while (m < (1u << 26) && ea > 1) { m <<= 1; ea--; }
    return sign | ((uint32_t)ea << 23) | ((m >> 3) & 0x7FFFFFu);
}

int32_t __aeabi_f2iz(uint32_t a) {
    uint32_t ua = a & 0x7FFFFFFFu;
    int32_t exp = (int32_t)(ua >> 23) - 127;
    if (exp < 0) return 0;
    if (exp >= 31) return (a & 0x80000000u) ? (-2147483647 - 1) : 2147483647;
    uint32_t m = (ua & 0x7FFFFFu) | 0x800000u;
    uint32_t v = (exp >= 23) ? (m << (exp - 23)) : (m >> (23 - exp));
    return (a & 0x80000000u) ? -(int32_t)v : (int32_t)v;
}

int __aeabi_fcmplt(uint32_t a, uint32_t b) {
    if (((a | b) & 0x7FFFFFFFu) == 0u) return 0;
    int32_t sa = (a & 0x80000000u) ? -(int32_t)(a & 0x7FFFFFFFu) : (int32_t)(a & 0x7FFFFFFFu);
    int32_t sb = (b & 0x80000000u) ? -(int32_t)(b & 0x7FFFFFFFu) : (int32_t)(b & 0x7FFFFFFFu);
    return sa < sb;
}

int __aeabi_fcmpge(uint32_t a, uint32_t b) {
    return !__aeabi_fcmplt(a, b);
}

uint64_t __aeabi_f2d(uint32_t a) {
    uint32_t ua = a & 0x7FFFFFFFu;
    if (ua == 0u) return ((uint64_t)(a & 0x80000000u)) << 32;
    uint64_t sign = ((uint64_t)(a & 0x80000000u)) << 32;
    uint64_t exp = (uint64_t)((ua >> 23) + (1023u - 127u)) << 52;
    uint64_t mant = ((uint64_t)(ua & 0x7FFFFFu)) << 29;
    return sign | exp | mant;
}

uint32_t __aeabi_d2f(uint64_t a) {
    uint32_t hi = (uint32_t)(a >> 32), lo = (uint32_t)a;
    uint32_t sign = hi & 0x80000000u;
    int32_t exp = (int32_t)((hi >> 20) & 0x7FFu) - (1023 - 127);
    if (exp <= 0) return sign;
    if (exp >= 255) return sign | 0x7F800000u;
    uint32_t mant = ((hi & 0xFFFFFu) << 3) | (lo >> 29);
    return sign | ((uint32_t)exp << 23) | mant;
}

uint64_t __aeabi_dsub(uint64_t a, uint64_t b) {
    return __aeabi_f2d(__aeabi_fsub(__aeabi_d2f(a), __aeabi_d2f(b)));
}

int __aeabi_dcmplt(uint64_t a, uint64_t b) {
    return __aeabi_fcmplt(__aeabi_d2f(a), __aeabi_d2f(b));
}

int __aeabi_dcmpge(uint64_t a, uint64_t b) {
    return !__aeabi_dcmplt(a, b);
}

int32_t __aeabi_d2iz(uint64_t a) {
    return __aeabi_f2iz(__aeabi_d2f(a));
}

uint64_t __aeabi_lmul(uint64_t a, uint64_t b) {
    uint32_t alo = (uint32_t)a, ahi = (uint32_t)(a >> 32);
    uint32_t blo = (uint32_t)b, bhi = (uint32_t)(b >> 32);
    uint32_t rlo, rhi;
    __asm__(
        "umull %0, %1, %2, %3\n\t"
        "mla   %1, %2, %5, %1\n\t"
        "mla   %1, %4, %3, %1"
        : "=&r"(rlo), "=&r"(rhi)
        : "r"(alo), "r"(blo), "r"(ahi), "r"(bhi)
    );
    return ((uint64_t)rhi << 32) | rlo;
}
