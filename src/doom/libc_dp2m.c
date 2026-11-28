/* SPDX-License-Identifier: GPL-2.0-or-later */
/* Freestanding C99 subset + Tracked Heap + MDF FILE Streams for DP2M DOOM.BIN */
#include <ctype.h>
#include <errno.h>
#include <stdarg.h>
#include <stdint.h>
#include <stdio.h>
#include <stdlib.h>
#include <string.h>
#include <sys/stat.h>
#include <unistd.h>
#include "dp2m_doom.h"

int errno = 0;

#define SMALL_FILE_PRELOAD_MAX 32768u

struct _dp2m_file {
    int32_t  fd;
    uint32_t pos;
    uint32_t size;
    uint8_t *ram_buf;
    uint8_t  in_use;
    uint8_t  eof;
    uint8_t  err;
    uint8_t  is_std;
};

static struct _dp2m_file g_std_files[3] = {
    { 0, 0, 0, NULL, 1, 0, 0, 1 },
    { 1, 0, 0, NULL, 1, 0, 0, 1 },
    { 2, 0, 0, NULL, 1, 0, 0, 1 },
};

FILE *stdin  = &g_std_files[0];
FILE *stdout = &g_std_files[1];
FILE *stderr = &g_std_files[2];

#define MAX_OPEN_FILES 8
#define MAX_RAW_FDS    16
#define LIBC_INIT_MAGIC 0x44324C43u /* "D2LC" */

static struct _dp2m_file g_files[MAX_OPEN_FILES];
static int32_t g_raw_fds[MAX_RAW_FDS];
static uint32_t g_libc_magic = 0u;

/* ---------- Tracked Heap Allocator (Zero-Leak Shutdown Guarantee) ---------- */

#define ALLOC_MAGIC 0x44324D41u /* "D2MA" */

struct alloc_hdr {
    struct alloc_hdr *prev;
    struct alloc_hdr *next;
    uint32_t size;
    uint32_t magic;
};

static struct alloc_hdr *g_alloc_head = NULL;

void dp2m_libc_reset(void) {
    if (g_libc_magic == LIBC_INIT_MAGIC) {
        dp2m_close_all_files();
        dp2m_free_all();
    }
    errno = 0;
    g_alloc_head = NULL;
    memset(g_files, 0, sizeof(g_files));
    for (int i = 0; i < MAX_RAW_FDS; i++)
        g_raw_fds[i] = -1;
    g_libc_magic = LIBC_INIT_MAGIC;
}

void *malloc(size_t size) {
    if (!g_dp2m_api || !g_dp2m_api->malloc_fn) return NULL;
    if (size == 0) size = 1;
    size_t total = sizeof(struct alloc_hdr) + ((size + 7u) & ~7u);
    if (total < size) return NULL;

    struct alloc_hdr *h = (struct alloc_hdr *)g_dp2m_api->malloc_fn(total);
    if (!h) {
        errno = ENOMEM;
        return NULL;
    }
    h->prev = NULL;
    h->next = g_alloc_head;
    h->size = (uint32_t)size;
    h->magic = ALLOC_MAGIC;
    if (g_alloc_head) g_alloc_head->prev = h;
    g_alloc_head = h;

    g_doom_tel.heap_bytes += (uint32_t)size;
    g_doom_tel.heap_blocks++;
    if (g_doom_tel.heap_bytes > g_doom_tel.heap_peak_bytes)
        g_doom_tel.heap_peak_bytes = g_doom_tel.heap_bytes;
    if (size >= 1024u * 1024u && g_doom_tel.zone_bytes == 0u)
        g_doom_tel.zone_bytes = (uint32_t)size;

    return (void *)(h + 1);
}

void free(void *ptr) {
    if (!ptr || !g_dp2m_api || !g_dp2m_api->free_fn) return;
    struct alloc_hdr *h = ((struct alloc_hdr *)ptr) - 1;
    if (h->magic != ALLOC_MAGIC) return;
    h->magic = 0u;

    if (h->prev) h->prev->next = h->next;
    else g_alloc_head = h->next;
    if (h->next) h->next->prev = h->prev;

    if (g_doom_tel.heap_bytes >= h->size) g_doom_tel.heap_bytes -= h->size;
    else g_doom_tel.heap_bytes = 0u;
    if (g_doom_tel.heap_blocks > 0u) g_doom_tel.heap_blocks--;

    g_dp2m_api->free_fn(h);
}

void *calloc(size_t nmemb, size_t size) {
    if (nmemb != 0 && size > ((size_t)-1) / nmemb) return NULL;
    size_t total = nmemb * size;
    void *p = malloc(total);
    if (p) memset(p, 0, total);
    return p;
}

void *realloc(void *ptr, size_t size) {
    if (!ptr) return malloc(size);
    if (size == 0) { free(ptr); return NULL; }
    struct alloc_hdr *h = ((struct alloc_hdr *)ptr) - 1;
    if (h->magic != ALLOC_MAGIC) return NULL;
    if (size <= h->size) return ptr;

    void *np = malloc(size);
    if (!np) return NULL;
    memcpy(np, ptr, h->size);
    free(ptr);
    return np;
}

void dp2m_free_all(void) {
    while (g_alloc_head) {
        void *payload = (void *)(g_alloc_head + 1);
        free(payload);
    }
    g_doom_tel.heap_bytes = 0u;
    g_doom_tel.heap_blocks = 0u;
}

/* ---------- String Functions ---------- */

size_t strlen(const char *s) {
    const char *p = s;
    while (*p) p++;
    return (size_t)(p - s);
}

char *strcpy(char *dst, const char *src) {
    char *d = dst;
    while ((*d++ = *src++) != '\0') {}
    return dst;
}

char *strncpy(char *dst, const char *src, size_t n) {
    size_t i = 0;
    for (; i < n && src[i] != '\0'; i++) dst[i] = src[i];
    for (; i < n; i++) dst[i] = '\0';
    return dst;
}

char *strcat(char *dst, const char *src) {
    char *d = dst + strlen(dst);
    strcpy(d, src);
    return dst;
}

char *strncat(char *dst, const char *src, size_t n) {
    char *d = dst + strlen(dst);
    size_t i = 0;
    for (; i < n && src[i] != '\0'; i++) d[i] = src[i];
    d[i] = '\0';
    return dst;
}

int strcmp(const char *a, const char *b) {
    while (*a && (*a == *b)) { a++; b++; }
    return (int)(unsigned char)*a - (int)(unsigned char)*b;
}

int strncmp(const char *a, const char *b, size_t n) {
    for (size_t i = 0; i < n; i++) {
        unsigned char ca = (unsigned char)a[i], cb = (unsigned char)b[i];
        if (ca != cb || ca == 0) return (int)ca - (int)cb;
    }
    return 0;
}

int strcasecmp(const char *a, const char *b) {
    for (;;) {
        int ca = tolower((unsigned char)*a++);
        int cb = tolower((unsigned char)*b++);
        if (ca != cb || ca == 0) return ca - cb;
    }
}

int strncasecmp(const char *a, const char *b, size_t n) {
    for (size_t i = 0; i < n; i++) {
        int ca = tolower((unsigned char)a[i]);
        int cb = tolower((unsigned char)b[i]);
        if (ca != cb || ca == 0) return ca - cb;
    }
    return 0;
}

char *strchr(const char *s, int c) {
    char ch = (char)c;
    for (; *s; s++) {
        if (*s == ch) return (char *)(uintptr_t)s;
    }
    return (ch == '\0') ? (char *)(uintptr_t)s : NULL;
}

char *strrchr(const char *s, int c) {
    const char *last = NULL;
    char ch = (char)c;
    for (; *s; s++) {
        if (*s == ch) last = s;
    }
    return (ch == '\0') ? (char *)(uintptr_t)s : (char *)(uintptr_t)last;
}

char *strstr(const char *haystack, const char *needle) {
    if (!*needle) return (char *)(uintptr_t)haystack;
    size_t nlen = strlen(needle);
    for (; *haystack; haystack++) {
        if (*haystack == *needle && strncmp(haystack, needle, nlen) == 0)
            return (char *)(uintptr_t)haystack;
    }
    return NULL;
}

char *strdup(const char *s) {
    size_t n = strlen(s) + 1u;
    char *p = (char *)malloc(n);
    if (p) memcpy(p, s, n);
    return p;
}

char *strerror(int errnum) {
    (void)errnum;
    return "error";
}

/* ---------- Conversions, Sort, Exit & Misc ---------- */

int abs(int x) { return x < 0 ? -x : x; }
long labs(long x) { return x < 0 ? -x : x; }

unsigned long strtoul(const char *nptr, char **endptr, int base) {
    const char *s = nptr;
    while (isspace((unsigned char)*s)) s++;
    int neg = 0;
    if (*s == '-') { neg = 1; s++; }
    else if (*s == '+') { s++; }

    if ((base == 0 || base == 16) && s[0] == '0' && (s[1] == 'x' || s[1] == 'X')) {
        base = 16;
        s += 2;
    } else if (base == 0) {
        base = (s[0] == '0') ? 8 : 10;
    }

    unsigned long acc = 0;
    const char *start = s;
    for (; *s; s++) {
        unsigned int d;
        unsigned char c = (unsigned char)*s;
        if (isdigit(c)) d = (unsigned int)(c - '0');
        else if (isupper(c)) d = (unsigned int)(c - 'A' + 10);
        else if (islower(c)) d = (unsigned int)(c - 'a' + 10);
        else break;
        if (d >= (unsigned int)base) break;
        acc = acc * (unsigned long)base + d;
    }
    if (endptr) *endptr = (char *)(uintptr_t)((s == start) ? nptr : s);
    return neg ? (0ul - acc) : acc;
}

long strtol(const char *nptr, char **endptr, int base) {
    const char *s = nptr;
    while (isspace((unsigned char)*s)) s++;
    int neg = 0;
    if (*s == '-') { neg = 1; s++; }
    else if (*s == '+') { s++; }
    if (*s == '-' || *s == '+' || isspace((unsigned char)*s)) {
        if (endptr) *endptr = (char *)(uintptr_t)nptr;
        return 0L;
    }
    char *ep = NULL;
    unsigned long u = strtoul(s, &ep, base);
    if (endptr) *endptr = (ep == s) ? (char *)(uintptr_t)nptr : ep;
    return neg ? -(long)u : (long)u;
}

int atoi(const char *s) { return (int)strtol(s, NULL, 10); }
long atol(const char *s) { return strtol(s, NULL, 10); }

double atof(const char *s) {
    if (!s) return 0.0;
    while (isspace((unsigned char)*s)) s++;
    int neg = 0;
    if (*s == '-') { neg = 1; s++; }
    else if (*s == '+') { s++; }
    float val = 0.0f;
    while (isdigit((unsigned char)*s)) {
        val = val * 10.0f + (float)(*s - '0');
        s++;
    }
    if (*s == '.') {
        s++;
        float scale = 0.1f;
        while (isdigit((unsigned char)*s)) {
            val += (float)(*s - '0') * scale;
            scale *= 0.1f;
            s++;
        }
    }
    if (neg) val = -val;
    return (double)val;
}

static uint32_t g_rand_seed = 1u;
int rand(void) {
    g_rand_seed = g_rand_seed * 1103515245u + 12345u;
    return (int)((g_rand_seed >> 16) & 0x7FFFu);
}
void srand(unsigned int seed) { g_rand_seed = seed; }

void qsort(void *base, size_t nmemb, size_t size,
           int (*compar)(const void *, const void *)) {
    uint8_t *arr = (uint8_t *)base;
    if (nmemb < 2 || size == 0) return;
    /* Shellsort (in-place, zero recursion stack risk on ARM9) */
    for (size_t gap = nmemb >> 1; gap > 0; gap >>= 1) {
        for (size_t i = gap; i < nmemb; i++) {
            size_t j = i;
            while (j >= gap) {
                uint8_t *a = arr + (j - gap) * size;
                uint8_t *b = arr + j * size;
                if (compar(a, b) <= 0) break;
                for (size_t k = 0; k < size; k++) {
                    uint8_t tmp = a[k];
                    a[k] = b[k];
                    b[k] = tmp;
                }
                j -= gap;
            }
        }
    }
}

void exit(int status) {
    if (g_doom_jmp_armed) {
        dp2m_longjmp(g_doom_exit_jmp, status == 0 ? 1 : -1);
    }
    for (;;) {}
}

void abort(void) { exit(-1); }
int atexit(void (*fn)(void)) { (void)fn; return 0; }
int system(const char *cmd) { (void)cmd; return -1; }
char *getenv(const char *name) { (void)name; return NULL; }
int mkdir(const char *path, mode_t mode) { (void)path; (void)mode; return 0; }
int remove(const char *path) { (void)path; return -1; }
int rename(const char *o, const char *n) { (void)o; (void)n; return -1; }
int isatty(int fd) { (void)fd; return 0; }
int usleep(unsigned int usec) {
    if (g_dp2m_api && g_dp2m_api->sleep_ms && usec >= 1000u)
        g_dp2m_api->sleep_ms(usec / 1000u);
    return 0;
}

/* ---------- MDF SD Card File Helpers & stdio Streams ---------- */

static void track_raw_fd(int32_t fd) {
    if (fd < 0) return;
    for (int i = 0; i < MAX_RAW_FDS; i++) {
        if (g_raw_fds[i] == fd) return;
    }
    for (int i = 0; i < MAX_RAW_FDS; i++) {
        if (g_raw_fds[i] < 0) {
            g_raw_fds[i] = fd;
            return;
        }
    }
}

void dp2m_sd_close(int32_t fd) {
    if (fd < 0) return;
    for (int i = 0; i < MAX_RAW_FDS; i++) {
        if (g_raw_fds[i] == fd)
            g_raw_fds[i] = -1;
    }
    if (g_dp2m_api && g_dp2m_api->mdf_close)
        g_dp2m_api->mdf_close(fd);
}

int32_t dp2m_sd_open_read(const char *path) {
    if (!path || !g_dp2m_api || !g_dp2m_api->mdf_open) return -1;
    while (path[0] == '.' && (path[1] == '/' || path[1] == '\\')) path += 2;
    while (path[0] == '/' || path[0] == '\\') path++;

    /* If explicit drive letter is already present, normalize slashes + uppercase */
    char norm[96];
    if (isalpha((unsigned char)path[0]) && path[1] == ':') {
        size_t i = 0;
        for (; path[i] && i + 1 < sizeof(norm); i++) {
            char c = path[i];
            norm[i] = (c == '/') ? '\\' : (char)toupper((unsigned char)c);
        }
        norm[i] = '\0';
        int32_t fd = g_dp2m_api->mdf_open(norm, 0u, 0u);
        if (fd >= 0) track_raw_fd(fd);
        return fd;
    }

    static const char drives[2] = { 'I', 'D' };
    for (int d = 0; d < 2; d++) {
        norm[0] = drives[d];
        norm[1] = ':';
        norm[2] = '\\';
        size_t o = 3;
        for (size_t i = 0; path[i] && o + 1 < sizeof(norm); i++) {
            char c = path[i];
            norm[o++] = (c == '/') ? '\\' : (char)toupper((unsigned char)c);
        }
        norm[o] = '\0';
        int32_t fd = g_dp2m_api->mdf_open(norm, 0u, 0u);
        if (fd >= 0) {
            track_raw_fd(fd);
            return fd;
        }
    }
    return -1;
}

static uint32_t mdf_probe_size(int32_t fd) {
    /* Stock mdf_fstat (0xA0111563) writes up to +0x224; allocate 0x240 bytes */
    uint8_t st[0x240] __attribute__((aligned(8)));
    memset(st, 0, sizeof(st));
    if (g_dp2m_api->mdf_fstat && g_dp2m_api->mdf_fstat(fd, st) == 0) {
        uint32_t sz = *(const uint32_t *)(const void *)(st + 0x214);
        if (sz > 0u) return sz;
    }
    uint32_t end_pos = 0u, zero_pos = 0u;
    if (g_dp2m_api->mdf_lseek && g_dp2m_api->mdf_lseek(fd, 0, SEEK_END, &end_pos) == 0) {
        g_dp2m_api->mdf_lseek(fd, 0, SEEK_SET, &zero_pos);
        return end_pos;
    }
    return 0u;
}

FILE *fopen(const char *path, const char *mode) {
    if (!path || !mode) return NULL;
    /* Read-only SD safety: reject write modes so config/save probes fail cleanly */
    if (strchr(mode, 'w') || strchr(mode, 'a') || strchr(mode, '+')) {
        errno = EACCES;
        return NULL;
    }
    int slot = -1;
    for (int i = 0; i < MAX_OPEN_FILES; i++) {
        if (!g_files[i].in_use) { slot = i; break; }
    }
    if (slot < 0) return NULL;

    int32_t fd = dp2m_sd_open_read(path);
    if (fd < 0) {
        errno = ENOENT;
        return NULL;
    }
    struct _dp2m_file *f = &g_files[slot];
    f->fd = fd;
    f->pos = 0u;
    f->size = mdf_probe_size(fd);
    f->ram_buf = NULL;
    f->in_use = 1u;
    f->eof = 0u;
    f->err = 0u;
    f->is_std = 0u;

    /* Preload small config/response files (<= 32 KB) so fgetc/fgets avoid per-byte SWI 10 */
    if (f->size > 0u && f->size <= SMALL_FILE_PRELOAD_MAX) {
        uint8_t *buf = (uint8_t *)malloc(f->size);
        if (buf) {
            uint32_t zero_pos = 0u;
            g_dp2m_api->mdf_lseek(fd, 0, SEEK_SET, &zero_pos);
            uint32_t got_tot = 0u;
            while (got_tot < f->size) {
                uint32_t got = 0u;
                uint32_t req = f->size - got_tot;
                if (g_dp2m_api->mdf_read(fd, buf + got_tot, req, &got) != 0 || got == 0u || got > req)
                    break;
                got_tot += got;
            }
            if (got_tot == f->size) {
                dp2m_sd_close(fd);
                f->fd = -1;
                f->ram_buf = buf;
            } else {
                free(buf);
                g_dp2m_api->mdf_lseek(fd, 0, SEEK_SET, &zero_pos);
            }
        }
    }
    return f;
}

int fclose(FILE *stream) {
    if (!stream || stream->is_std || !stream->in_use) return 0;
    if (stream->fd >= 0) {
        dp2m_sd_close(stream->fd);
        stream->fd = -1;
    }
    if (stream->ram_buf) {
        free(stream->ram_buf);
        stream->ram_buf = NULL;
    }
    stream->in_use = 0u;
    return 0;
}

void dp2m_close_all_files(void) {
    for (int i = 0; i < MAX_OPEN_FILES; i++) {
        if (g_files[i].in_use) fclose(&g_files[i]);
    }
    for (int i = 0; i < MAX_RAW_FDS; i++) {
        if (g_raw_fds[i] >= 0)
            dp2m_sd_close(g_raw_fds[i]);
    }
}

size_t fread(void *ptr, size_t size, size_t nmemb, FILE *stream) {
    if (!ptr || !stream || stream->is_std || !stream->in_use || size == 0 || nmemb == 0)
        return 0;
    if (nmemb > ((size_t)-1) / size)
        return 0;
    size_t want = size * nmemb;
    uint8_t *dst = (uint8_t *)ptr;

    if (stream->ram_buf) {
        size_t avail = (stream->pos < stream->size) ? (size_t)(stream->size - stream->pos) : 0u;
        size_t n = (want < avail) ? want : avail;
        if (n > 0) {
            memcpy(dst, stream->ram_buf + stream->pos, n);
            stream->pos += (uint32_t)n;
        }
        if (n < want)
            stream->eof = 1u;
        return n / size;
    }

    if (stream->fd < 0) {
        stream->eof = 1u;
        return 0;
    }

    size_t total = 0;
    while (total < want) {
        uint32_t chunk = (want - total > 65536u) ? 65536u : (uint32_t)(want - total);
        uint32_t got = 0u;
        int32_t rc = g_dp2m_api->mdf_read(stream->fd, dst + total, chunk, &got);
        if (rc != 0 || got > chunk) { stream->err = 1u; break; }
        if (got == 0u) { stream->eof = 1u; break; }
        total += got;
        stream->pos += got;
        if (got < chunk) { stream->eof = 1u; break; }
    }
    return total / size;
}

size_t fwrite(const void *ptr, size_t size, size_t nmemb, FILE *stream) {
    (void)ptr;
    (void)size;
    if (!stream) return 0;
    if (stream->is_std) return nmemb;
    return 0;
}

int fseek(FILE *stream, long offset, int whence) {
    if (!stream || stream->is_std || !stream->in_use) return -1;
    if (stream->ram_buf) {
        int64_t base;
        if (whence == SEEK_SET) base = 0;
        else if (whence == SEEK_CUR) base = (int64_t)stream->pos;
        else if (whence == SEEK_END) base = (int64_t)stream->size;
        else { stream->err = 1u; return -1; }
        int64_t target = base + (int64_t)offset;
        if (target < 0 || target > (int64_t)stream->size) {
            stream->err = 1u;
            return -1;
        }
        stream->pos = (uint32_t)target;
        stream->eof = 0u;
        return 0;
    }
    if (stream->fd < 0) return -1;
    uint32_t new_pos = 0u;
    if (g_dp2m_api->mdf_lseek(stream->fd, (int32_t)offset, (uint32_t)whence, &new_pos) != 0) {
        stream->err = 1u;
        return -1;
    }
    stream->pos = new_pos;
    stream->eof = 0u;
    return 0;
}

long ftell(FILE *stream) {
    if (!stream || stream->is_std || !stream->in_use) return -1L;
    return (long)stream->pos;
}

void rewind(FILE *stream) {
    if (stream) fseek(stream, 0L, SEEK_SET);
}

int fflush(FILE *stream) { (void)stream; return 0; }
int feof(FILE *stream)   { return stream ? (int)stream->eof : 1; }
int ferror(FILE *stream) { return stream ? (int)stream->err : 1; }

int fgetc(FILE *stream) {
    unsigned char c = 0;
    return (fread(&c, 1, 1, stream) == 1) ? (int)c : EOF;
}
int getc(FILE *stream) { return fgetc(stream); }
int fputc(int c, FILE *stream) { (void)stream; return (unsigned char)c; }
int putc(int c, FILE *stream) { (void)stream; return (unsigned char)c; }
int putchar(int c) { return (unsigned char)c; }
int fputs(const char *s, FILE *stream) { (void)s; (void)stream; return 0; }
int puts(const char *s) { (void)s; return 0; }

char *fgets(char *s, int size, FILE *stream) {
    if (!s || size <= 0 || !stream) return NULL;
    int i = 0;
    while (i + 1 < size) {
        int c = fgetc(stream);
        if (c == EOF) {
            if (i == 0) return NULL;
            break;
        }
        s[i++] = (char)c;
        if (c == '\n') break;
    }
    s[i] = '\0';
    return s;
}

/* ---------- Compact vsnprintf / snprintf / sscanf ---------- */

static void out_ch(char *buf, size_t size, size_t *idx, char c) {
    if (buf && *idx + 1 < size) buf[*idx] = c;
    (*idx)++;
}

static uint32_t udiv64_base(uint64_t *val, uint32_t base) {
    uint32_t hi = (uint32_t)(*val >> 32);
    uint32_t lo = (uint32_t)(*val);
    if (hi == 0u) {
        uint32_t r = lo % base;
        *val = lo / base;
        return r;
    }
    uint32_t q_hi = hi / base;
    uint32_t r = hi % base;
    uint32_t m1 = (r << 16) | (lo >> 16);
    uint32_t q_m1 = m1 / base;
    r = m1 % base;
    uint32_t m0 = (r << 16) | (lo & 0xFFFFu);
    uint32_t q_m0 = m0 / base;
    r = m0 % base;
    *val = ((uint64_t)q_hi << 32) | ((uint64_t)q_m1 << 16) | q_m0;
    return r;
}

int vsnprintf(char *str, size_t size, const char *fmt, va_list ap) {
    size_t idx = 0;
    for (const char *p = fmt; *p; p++) {
        if (*p != '%') {
            out_ch(str, size, &idx, *p);
            continue;
        }
        p++;
        if (*p == '%') { out_ch(str, size, &idx, '%'); continue; }

        int left = 0, plus = 0, space = 0, alt = 0;
        char pad = ' ';
        while (*p == '-' || *p == '0' || *p == '+' || *p == ' ' || *p == '#') {
            if (*p == '-') { left = 1; pad = ' '; }
            else if (*p == '0' && !left) pad = '0';
            else if (*p == '+') plus = 1;
            else if (*p == ' ') space = 1;
            else if (*p == '#') alt = 1;
            p++;
        }
        int width = 0;
        if (*p == '*') {
            width = va_arg(ap, int);
            if (width < 0) { left = 1; pad = ' '; width = -width; }
            p++;
        } else {
            while (isdigit((unsigned char)*p)) width = width * 10 + (*p++ - '0');
        }

        int prec = -1;
        if (*p == '.') {
            p++;
            prec = 0;
            if (*p == '*') { prec = va_arg(ap, int); p++; }
            else while (isdigit((unsigned char)*p)) { prec = prec * 10 + (*p++ - '0'); }
        }

        int lcnt = 0;
        while (*p == 'l' || *p == 'h' || *p == 'z') {
            if (*p == 'l') lcnt++;
            p++;
        }

        char spec = *p;
        if (!spec) break;

        if (spec == 'c') {
            char c = (char)va_arg(ap, int);
            if (!left) for (int w = 1; w < width; w++) out_ch(str, size, &idx, ' ');
            out_ch(str, size, &idx, c);
            if (left) for (int w = 1; w < width; w++) out_ch(str, size, &idx, ' ');
        } else if (spec == 's') {
            const char *s = va_arg(ap, const char *);
            if (!s) s = "(null)";
            int len = (int)strlen(s);
            if (prec >= 0 && prec < len) len = prec;
            if (!left) for (int w = len; w < width; w++) out_ch(str, size, &idx, ' ');
            for (int i = 0; i < len; i++) out_ch(str, size, &idx, s[i]);
            if (left) for (int w = len; w < width; w++) out_ch(str, size, &idx, ' ');
        } else if (spec == 'd' || spec == 'i' || spec == 'u' ||
                   spec == 'x' || spec == 'X' || spec == 'p' || spec == 'o') {
            uint64_t u = 0;
            char sign_ch = 0;
            uint32_t base = 10u;
            int upper = (spec == 'X');
            int hex_pfx = 0;
            if (spec == 'd' || spec == 'i') {
                int64_t v = (lcnt >= 2) ? va_arg(ap, long long) : (int64_t)va_arg(ap, int32_t);
                if (v < 0) { sign_ch = '-'; u = 0ull - (uint64_t)v; }
                else {
                    u = (uint64_t)v;
                    if (plus) sign_ch = '+';
                    else if (space) sign_ch = ' ';
                }
            } else if (spec == 'p') {
                u = (uint64_t)(uintptr_t)va_arg(ap, void *);
                base = 16u;
                hex_pfx = 1;
            } else {
                u = (lcnt >= 2) ? va_arg(ap, unsigned long long) : (uint64_t)va_arg(ap, uint32_t);
                if (spec == 'x' || spec == 'X') { base = 16u; hex_pfx = (alt && u != 0ull); }
                else if (spec == 'o') base = 8u;
            }
            char tmp[24];
            int n = 0;
            do {
                uint32_t d = udiv64_base(&u, base);
                tmp[n++] = (char)(d < 10u ? ('0' + d) : ((upper ? 'A' : 'a') + d - 10u));
            } while (u != 0ull && n < (int)sizeof(tmp));
            while (n < prec && n < (int)sizeof(tmp)) tmp[n++] = '0';
            int pfx_len = (sign_ch ? 1 : 0) + (hex_pfx ? 2 : 0);
            int total = n + pfx_len;
            if (!left && pad == ' ')
                for (int w = total; w < width; w++) out_ch(str, size, &idx, ' ');
            if (sign_ch) out_ch(str, size, &idx, sign_ch);
            if (hex_pfx) {
                out_ch(str, size, &idx, '0');
                out_ch(str, size, &idx, upper ? 'X' : 'x');
            }
            if (!left && pad == '0')
                for (int w = total; w < width; w++) out_ch(str, size, &idx, '0');
            for (int i = n - 1; i >= 0; i--) out_ch(str, size, &idx, tmp[i]);
            if (left)
                for (int w = total; w < width; w++) out_ch(str, size, &idx, ' ');
        } else if (spec == 'f') {
            double dv = va_arg(ap, double);
            int32_t iv = (int32_t)dv;
            char tmp[16];
            int n = snprintf(tmp, sizeof(tmp), "%d", (int)iv);
            for (int i = 0; i < n; i++) out_ch(str, size, &idx, tmp[i]);
        } else {
            out_ch(str, size, &idx, spec);
        }
    }
    if (str && size > 0) {
        str[idx < size ? idx : size - 1] = '\0';
    }
    return (int)idx;
}

int snprintf(char *str, size_t size, const char *fmt, ...) {
    va_list ap;
    va_start(ap, fmt);
    int r = vsnprintf(str, size, fmt, ap);
    va_end(ap);
    return r;
}

int vsprintf(char *str, const char *fmt, va_list ap) {
    return vsnprintf(str, 0x7FFFFFFFu, fmt, ap);
}

int sprintf(char *str, const char *fmt, ...) {
    va_list ap;
    va_start(ap, fmt);
    int r = vsnprintf(str, 0x7FFFFFFFu, fmt, ap);
    va_end(ap);
    return r;
}

int vfprintf(FILE *stream, const char *fmt, va_list ap) {
    if (stream == stderr && fmt && *fmt && *fmt != '\n') {
        vsnprintf(g_doom_tel.last_err, sizeof(g_doom_tel.last_err), fmt, ap);
        return (int)strlen(g_doom_tel.last_err);
    }
    (void)stream;
    (void)fmt;
    (void)ap;
    return 0;
}

int vprintf(const char *fmt, va_list ap) {
    return vfprintf(stdout, fmt, ap);
}

int fprintf(FILE *stream, const char *fmt, ...) {
    va_list ap;
    va_start(ap, fmt);
    int r = vfprintf(stream, fmt, ap);
    va_end(ap);
    return r;
}

int printf(const char *fmt, ...) {
    va_list ap;
    va_start(ap, fmt);
    int r = vfprintf(stdout, fmt, ap);
    va_end(ap);
    return r;
}

int sscanf(const char *str, const char *fmt, ...) {
    if (!str || !fmt) return 0;
    va_list ap;
    va_start(ap, fmt);
    int matched = 0;
    const char *s = str;
    for (const char *f = fmt; *f; f++) {
        if (isspace((unsigned char)*f)) {
            while (isspace((unsigned char)*s)) s++;
            continue;
        }
        if (*f != '%') {
            if (*s != *f) break;
            s++;
            continue;
        }
        f++;
        while (isspace((unsigned char)*s)) s++;
        if (!*s) break;
        if (*f == 'x' || *f == 'X' || *f == 'i' || *f == 'd' || *f == 'u') {
            int base = (*f == 'x' || *f == 'X') ? 16 : (*f == 'i' ? 0 : 10);
            char *ep = NULL;
            long val = strtol(s, &ep, base);
            if (ep == s) break;
            int *out = va_arg(ap, int *);
            if (out) *out = (int)val;
            s = ep;
            matched++;
        } else if (*f == 's') {
            char *out = va_arg(ap, char *);
            while (*s && !isspace((unsigned char)*s)) {
                if (out) *out++ = *s;
                s++;
            }
            if (out) *out = '\0';
            matched++;
        } else {
            break;
        }
    }
    va_end(ap);
    return matched;
}
