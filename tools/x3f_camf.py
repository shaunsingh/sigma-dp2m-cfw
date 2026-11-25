#!/usr/bin/env python3
"""Decode Sigma Merrill X3F Type-4 Huffman CAMF sections & dump all CMbM/CMbP/CMbT entries."""
import json, struct, sys

def decode_camf_type4(raw: bytes) -> bytes:
    magic, ver, ctype, dec_size, bias, cols, rows = struct.unpack('<4sIIIIII', raw[:28])
    assert magic == b'SECc' and ctype == 4, f"Unexpected CAMF header: {magic} type={ctype}"

    # Build prefix lookup table from (code_size, top_aligned_code) pairs starting at offset 28
    # Note: Kalpanika reads 2-byte pairs until code_size == 0; data_size is at raw[28 + 28] = offset 56 (0x38)!
    pos = 28
    tree = {}
    idx = 0
    while raw[pos] != 0:
        blen, code = raw[pos], raw[pos + 1]
        tree[(blen, code >> (8 - blen))] = idx
        idx += 1
        pos += 2
    max_len = max(k[0] for k in tree)

    # Per Kalpanika x3f_io.c: CAMF_T4_DATA_SIZE_OFFSET = 28 (from CAMF->data = raw+28), DATA_OFFSET = 32
    data_start = 28 + 32
    bs = raw[data_start:]
    n = len(bs)

    # Fast lookup table indexed by top 8 bits -> (sym, blen)
    lut = [None] * 256
    for (blen, cval), sym in tree.items():
        shift = 8 - blen
        base = cval << shift
        for pad in range(1 << shift):
            lut[base | pad] = (sym, blen)

    out = bytearray(dec_size)
    dst = 0
    odd_dst = 0
    row_start_acc = [[bias, bias], [bias, bias]]

    bitbuf = 0
    nbits = 0
    bi = 0

    for row in range(rows):
        odd_row = row & 1
        acc = [0, 0]
        racc = row_start_acc[odd_row]
        for col in range(cols):
            odd_col = col & 1
            while nbits < 8 and bi < n:
                bitbuf = (bitbuf << 8) | bs[bi]
                bi += 1
                nbits += 8
            top8 = (bitbuf >> (nbits - 8)) & 0xff
            sym, blen = lut[top8]
            nbits -= blen
            bitbuf &= (1 << nbits) - 1

            if sym == 0:
                diff = 0
            else:
                while nbits < sym and bi < n:
                    bitbuf = (bitbuf << 8) | bs[bi]
                    bi += 1
                    nbits += 8
                val = (bitbuf >> (nbits - sym)) & ((1 << sym) - 1)
                nbits -= sym
                bitbuf &= (1 << nbits) - 1
                if val < (1 << (sym - 1)):
                    val -= (1 << sym) - 1
                diff = val

            prev = racc[odd_col] if col < 2 else acc[odd_col]
            value = prev + diff
            acc[odd_col] = value
            if col < 2:
                racc[odd_col] = value

            if odd_dst == 0:
                out[dst] = (value >> 4) & 0xff
                dst += 1
                if dst >= dec_size:
                    return bytes(out)
                out[dst] = (value << 4) & 0xf0
            else:
                out[dst] |= (value >> 8) & 0x0f
                dst += 1
                if dst >= dec_size:
                    return bytes(out)
                out[dst] = value & 0xff
                dst += 1
                if dst >= dec_size:
                    return bytes(out)
            odd_dst ^= 1
    return bytes(out)


def extract_x3f_camf(x3f_path: str) -> bytes:
    with open(x3f_path, 'rb') as f:
        f.seek(0, 2)
        sz = f.tell()
        f.seek(sz - 4)
        doff = struct.unpack('<I', f.read(4))[0]
        f.seek(doff)
        _, _, dcnt = struct.unpack('<4sII', f.read(12))
        for _ in range(dcnt):
            soff, slen, stype = struct.unpack('<II4s', f.read(12))
            if stype == b'CAMF':
                f.seek(soff)
                return decode_camf_type4(f.read(slen))
    raise ValueError("No CAMF section found")


def parse_camf(buf: bytes) -> dict:
    entries = {}
    pos = 0
    n = len(buf)
    while pos + 20 <= n:
        tag = buf[pos:pos+4]
        if tag not in (b'CMbM', b'CMbP', b'CMbT'):
            break
        ver, esz, noff, voff = struct.unpack('<4I', buf[pos+4:pos+20])
        if esz == 0 or pos + esz > n:
            break
        z = buf.find(b'\x00', pos + noff, pos + esz)
        name = buf[pos+noff:z].decode('ascii', 'ignore') if z > pos + noff else f"unk_{pos:x}"
        if tag == b'CMbT':
            tlen = struct.unpack('<I', buf[pos+voff:pos+voff+4])[0]
            val = buf[pos+voff+4:pos+voff+4+tlen].decode('ascii', 'ignore').rstrip('\x00')
            entries[name] = ('T', val)
        elif tag == b'CMbP':
            cnt, doff = struct.unpack('<II', buf[pos+voff:pos+voff+8])
            props = {}
            for i in range(cnt):
                ko, vo = struct.unpack('<II', buf[pos+voff+8+i*8:pos+voff+16+i*8])
                kz = buf.find(b'\x00', pos+doff+ko, pos+esz)
                vz = buf.find(b'\x00', pos+doff+vo, pos+esz)
                k = buf[pos+doff+ko:kz].decode('ascii', 'ignore')
                v = buf[pos+doff+vo:vz].decode('ascii', 'ignore')
                props[k] = v
            entries[name] = ('P', props)
        elif tag == b'CMbM':
            dtype, ndim, doff = struct.unpack('<3I', buf[pos+voff:pos+voff+12])
            dims = [struct.unpack('<3I', buf[pos+voff+12+d*12:pos+voff+24+d*12])[0] for d in range(ndim)]
            cnt = 1
            for d in dims: cnt *= d
            dpos = pos + doff
            fmt_map = {0: ('h', 2), 1: ('i', 4), 2: ('I', 4), 3: ('f', 4), 5: ('B', 1), 6: ('H', 2)}
            if dtype in fmt_map and cnt <= 256:
                fc, fs = fmt_map[dtype]
                vals = list(struct.unpack(f'<{cnt}{fc}', buf[dpos:dpos+cnt*fs]))
                if fc == 'f':
                    vals = [round(x, 6) for x in vals]
                entries[name] = ('M', {'dtype': dtype, 'dims': dims, 'vals': vals})
            else:
                entries[name] = ('M', {'dtype': dtype, 'dims': dims, 'size': cnt})
        pos += esz
    return entries


if __name__ == '__main__':
    x3f = sys.argv[1]
    raw_camf = extract_x3f_camf(x3f)
    if len(sys.argv) > 2:
        open(sys.argv[2], 'wb').write(raw_camf)
    entries = parse_camf(raw_camf)
    print(f"Decoded {len(raw_camf)} bytes, parsed {len(entries)} CAMF entries from {x3f}")
    for k, (t, v) in entries.items():
        if any(w in k for w in ['Color', 'CMCM', 'CMCC', 'MCAS', 'CCMatrix', 'WBGain', 'TC', 'Gamma', 'Sat', 'CP1', 'CP2', 'Camsettings', 'Model', 'Firmware', 'Sharp', 'Contrast', 'Noise', 'Mode', 'Capture', 'WhiteBalance']):
            print(f"  [{t}] {k:34s}: {str(v)[:140]}")
