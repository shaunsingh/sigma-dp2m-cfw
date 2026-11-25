/* Minimal TRUE-II Merrill X3F raw decoder.
 * Decodes type=1 fmt=30 (4928x3264x3) to interleaved uint16_t raw binary (H x W x 3 LE)
 * with a 16-byte header: magic "X3R\0", u32 width, u32 height, u32 channels.
 */
#include <stdint.h>
#include <stdio.h>
#include <stdlib.h>
#include <string.h>

static uint16_t rd16(const uint8_t *p) { return (uint16_t)p[0] | ((uint16_t)p[1] << 8); }
static uint32_t rd32(const uint8_t *p) {
    return (uint32_t)p[0] | ((uint32_t)p[1] << 8) | ((uint32_t)p[2] << 16) | ((uint32_t)p[3] << 24);
}

typedef struct {
    int branch[2];
    int leaf;
} Node;

typedef struct {
    Node nodes[512];
    int count;
} Tree;

static int new_node(Tree *t) {
    int id = t->count++;
    t->nodes[id].branch[0] = -1;
    t->nodes[id].branch[1] = -1;
    t->nodes[id].leaf = -1;
    return id;
}

static void add_code(Tree *t, int len, uint32_t code, int val) {
    int cur = 0;
    for (int i = 0; i < len; i++) {
        int bit = (code >> (len - 1 - i)) & 1;
        if (t->nodes[cur].branch[bit] < 0)
            t->nodes[cur].branch[bit] = new_node(t);
        cur = t->nodes[cur].branch[bit];
    }
    t->nodes[cur].leaf = val;
}

typedef struct {
    const uint8_t *p;
    uint32_t cur;
    int bits_left;
} BitStream;

static inline uint32_t get_bit(BitStream *bs) {
    if (bs->bits_left == 0) {
        bs->cur = *bs->p++;
        bs->bits_left = 8;
    }
    bs->bits_left--;
    return (bs->cur >> bs->bits_left) & 1u;
}

static inline int32_t get_true_diff(BitStream *bs, const Tree *t) {
    int cur = 0;
    while (t->nodes[cur].branch[0] >= 0 || t->nodes[cur].branch[1] >= 0) {
        cur = t->nodes[cur].branch[get_bit(bs)];
        if (cur < 0) return 0;
    }
    int bits = t->nodes[cur].leaf;
    if (bits <= 0) return 0;
    uint32_t first = get_bit(bs);
    int32_t diff = (int32_t)first;
    for (int i = 1; i < bits; i++)
        diff = (diff << 1) | (int32_t)get_bit(bs);
    if (first == 0)
        diff -= (1 << bits) - 1;
    return diff;
}

int main(int argc, char **argv) {
    if (argc != 3) {
        fprintf(stderr, "Usage: %s input.X3F output.x3r\n", argv[0]);
        return 1;
    }
    FILE *f = fopen(argv[1], "rb");
    if (!f) { perror("open input"); return 1; }
    fseek(f, 0, SEEK_END);
    size_t fsz = (size_t)ftell(f);
    fseek(f, 0, SEEK_SET);
    uint8_t *buf = (uint8_t *)malloc(fsz);
    if (!buf || fread(buf, 1, fsz, f) != fsz) { fprintf(stderr, "read fail\n"); return 1; }
    fclose(f);

    uint32_t dir_off = rd32(buf + fsz - 4);
    uint32_t nsec = rd32(buf + dir_off + 8);
    uint32_t sec_off = 0, sec_len = 0;
    uint32_t cols = 0, rows = 0;
    for (uint32_t i = 0; i < nsec; i++) {
        const uint8_t *e = buf + dir_off + 12 + i * 12;
        uint32_t off = rd32(e), len = rd32(e + 4);
        if (memcmp(e + 8, "IMA2", 4) == 0 || memcmp(e + 8, "IMAG", 4) == 0) {
            uint32_t itype = rd32(buf + off + 8);
            uint32_t dfmt  = rd32(buf + off + 12);
            if (itype == 1 && dfmt == 30) {
                sec_off = off;
                sec_len = len;
                cols = rd32(buf + off + 16);
                rows = rd32(buf + off + 20);
                break;
            }
        }
    }
    if (!sec_off) { fprintf(stderr, "No type=1 fmt=30 raw section found\n"); return 1; }

    const uint8_t *p = buf + sec_off + 28;
    uint16_t seed[3];
    seed[0] = rd16(p); seed[1] = rd16(p + 2); seed[2] = rd16(p + 4);
    p += 8; /* skip seeds[3] + unknown[1] */

    Tree tree = {0};
    new_node(&tree);
    for (int i = 0; ; i++) {
        uint8_t code_size = *p++;
        uint8_t code = *p++;
        if (code_size == 0) break;
        add_code(&tree, code_size, (uint32_t)(code >> (8 - code_size)) & 0xffu, i);
    }

    uint32_t plane_sz[3];
    plane_sz[0] = rd32(p); plane_sz[1] = rd32(p + 4); plane_sz[2] = rd32(p + 8);
    p += 12;

    const uint8_t *plane_ptr[3];
    plane_ptr[0] = p;
    plane_ptr[1] = plane_ptr[0] + (((plane_sz[0] + 15) / 16) * 16);
    plane_ptr[2] = plane_ptr[1] + (((plane_sz[1] + 15) / 16) * 16);

    size_t npix = (size_t)rows * cols;
    uint16_t *out = (uint16_t *)malloc(npix * 3 * sizeof(uint16_t));
    if (!out) { fprintf(stderr, "oom\n"); return 1; }

    for (int c = 0; c < 3; c++) {
        BitStream bs = { plane_ptr[c], 0, 0 };
        int32_t row_start[2][2] = { {seed[c], seed[c]}, {seed[c], seed[c]} };
        uint16_t *dst = out + c;
        for (uint32_t r = 0; r < rows; r++) {
            int odd_r = r & 1;
            int32_t acc[2];
            for (uint32_t x = 0; x < cols; x++) {
                int odd_x = x & 1;
                int32_t diff = get_true_diff(&bs, &tree);
                int32_t prev = (x < 2) ? row_start[odd_r][odd_x] : acc[odd_x];
                int32_t val = prev + diff;
                acc[odd_x] = val;
                if (x < 2) row_start[odd_r][odd_x] = val;
                *dst = (uint16_t)(val < 0 ? 0 : (val > 65535 ? 65535 : val));
                dst += 3;
            }
        }
    }

    FILE *of = fopen(argv[2], "wb");
    if (!of) { perror("open output"); return 1; }
    uint32_t hdr[4] = { 0x00523358u, cols, rows, 3u };
    fwrite(hdr, 4, 4, of);
    fwrite(out, sizeof(uint16_t), npix * 3, of);
    fclose(of);
    printf("Decoded %s -> %s (%ux%ux3, seeds=%u,%u,%u, planes=%u,%u,%u)\n",
           argv[1], argv[2], cols, rows, seed[0], seed[1], seed[2],
           plane_sz[0], plane_sz[1], plane_sz[2]);
    free(out);
    free(buf);
    return 0;
}
