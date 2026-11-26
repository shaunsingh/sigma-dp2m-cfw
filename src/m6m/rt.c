// freestanding libc / AEABI helpers; ARMv5TE big-endian requires 4-byte alignment for word accesses
#include <stddef.h>
#include <stdint.h>

void *memset(void *d, int c, size_t n) {
    uint8_t *p = d, b = (uint8_t)c;
    if (n >= 4) {
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

void *memcpy(void *d, const void *s, size_t n) {
    uint8_t *p = d;
    const uint8_t *q = s;
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

void __aeabi_memset(void *d, size_t n, int c) { memset(d, c, n); }
void __aeabi_memset4(void *d, size_t n, int c) __attribute__((alias("__aeabi_memset")));
void __aeabi_memclr(void *d, size_t n) { memset(d, 0, n); }
void __aeabi_memclr4(void *d, size_t n) __attribute__((alias("__aeabi_memclr")));
void __aeabi_memcpy(void *d, const void *s, size_t n) { memcpy(d, s, n); }
void __aeabi_memcpy4(void *d, const void *s, size_t n) __attribute__((alias("__aeabi_memcpy")));
