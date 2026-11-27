//! Sigma Merrill X3F Type-4 Huffman CAMF decompressor and CMbT/CMbP/CMbM parser (`tools/x3f_camf.py`).

#[derive(Clone, Debug, PartialEq)]
pub enum MatrixVals {
    I16(Vec<i16>),
    I32(Vec<i32>),
    U32(Vec<u32>),
    F32(Vec<f32>),
    U8(Vec<u8>),
    U16(Vec<u16>),
    Large(usize),
}

#[derive(Clone, Debug, PartialEq)]
pub enum CamfValue {
    Text(String),
    Props(Vec<(String, String)>),
    Matrix { dtype: u32, dims: Vec<u32>, vals: MatrixVals },
}

#[derive(Clone, Debug, PartialEq)]
pub struct CamfEntry {
    pub name: String,
    pub value: CamfValue,
}

#[inline]
fn le32(b: &[u8], o: usize) -> u32 {
    u32::from_le_bytes([b[o], b[o + 1], b[o + 2], b[o + 3]])
}

/// Decompresses an X3F `SECc` Type-4 Huffman-coded CAMF section.
pub fn decode_camf_type4(raw: &[u8]) -> Result<Vec<u8>, String> {
    if raw.len() < 60 || &raw[..4] != b"SECc" {
        return Err("invalid CAMF section header".into());
    }
    let ctype = le32(raw, 8);
    if ctype != 4 {
        return Err(format!("unsupported CAMF type {ctype} (expected 4)"));
    }
    let dec_size = le32(raw, 12) as usize;
    let bias = le32(raw, 16) as i32;
    let cols = le32(raw, 20) as usize;
    let rows = le32(raw, 24) as usize;

    // 2-byte (bit_len, top_aligned_code) table at offset 28, terminated by bit_len == 0
    let mut lut = [(0u8, 0u8); 256]; // (sym, blen)
    let mut pos = 28usize;
    let mut sym = 0u8;
    while pos + 1 < raw.len() && raw[pos] != 0 {
        let blen = raw[pos];
        let code = raw[pos + 1];
        if blen > 8 {
            return Err(format!("CAMF Huffman code length {blen} > 8"));
        }
        let shift = 8 - blen;
        let base = (code >> shift) << shift;
        for pad in 0..(1u16 << shift) {
            lut[(base as usize) | (pad as usize)] = (sym, blen);
        }
        sym = sym.wrapping_add(1);
        pos += 2;
    }

    // Kalpanika x3f_io.c: CAMF_T4_DATA_OFFSET = 32 from raw+28
    let bs = &raw[60..];
    let mut out = vec![0u8; dec_size];
    let mut dst = 0usize;
    let mut odd_dst = false;
    let mut row_start = [[bias; 2]; 2];

    let mut bitbuf = 0u64;
    let mut nbits = 0u32;
    let mut bi = 0usize;

    for row in 0..rows {
        let mut acc = [0i32; 2];
        let racc = &mut row_start[row & 1];
        for col in 0..cols {
            let odd_col = col & 1;
            while nbits < 8 && bi < bs.len() {
                bitbuf = (bitbuf << 8) | (bs[bi] as u64);
                bi += 1;
                nbits += 8;
            }
            if nbits < 8 {
                return Ok(out);
            }
            let top8 = ((bitbuf >> (nbits - 8)) & 0xFF) as usize;
            let (s, blen) = lut[top8];
            if blen == 0 {
                return Err(format!("invalid CAMF Huffman code {top8:#04x} at ({row},{col})"));
            }
            nbits -= blen as u32;
            bitbuf &= (1u64 << nbits) - 1;

            let diff = if s == 0 {
                0i32
            } else {
                let sb = s as u32;
                while nbits < sb && bi < bs.len() {
                    bitbuf = (bitbuf << 8) | (bs[bi] as u64);
                    bi += 1;
                    nbits += 8;
                }
                let mut val = ((bitbuf >> (nbits - sb)) & ((1u64 << sb) - 1)) as i32;
                nbits -= sb;
                bitbuf &= (1u64 << nbits) - 1;
                if val < (1 << (sb - 1)) {
                    val -= (1 << sb) - 1;
                }
                val
            };

            let prev = if col < 2 { racc[odd_col] } else { acc[odd_col] };
            let value = prev.wrapping_add(diff);
            acc[odd_col] = value;
            if col < 2 {
                racc[odd_col] = value;
            }

            if !odd_dst {
                out[dst] = ((value >> 4) & 0xFF) as u8;
                dst += 1;
                if dst >= dec_size {
                    return Ok(out);
                }
                out[dst] = ((value << 4) & 0xF0) as u8;
            } else {
                out[dst] |= ((value >> 8) & 0x0F) as u8;
                dst += 1;
                if dst >= dec_size {
                    return Ok(out);
                }
                out[dst] = (value & 0xFF) as u8;
                dst += 1;
                if dst >= dec_size {
                    return Ok(out);
                }
            }
            odd_dst = !odd_dst;
        }
    }
    Ok(out)
}

/// Locates and decompresses the `CAMF` directory section inside an X3F raw file.
pub fn extract_x3f_camf(x3f: &[u8]) -> Result<Vec<u8>, String> {
    if x3f.len() < 16 || &x3f[..4] != b"FOVb" {
        return Err("not an X3F file (missing FOVb magic)".into());
    }
    let doff = le32(x3f, x3f.len() - 4) as usize;
    if doff + 12 > x3f.len() || &x3f[doff..doff + 4] != b"SECd" {
        return Err("invalid X3F directory offset".into());
    }
    let dcnt = le32(x3f, doff + 8) as usize;
    for i in 0..dcnt {
        let e = doff + 12 + i * 12;
        if e + 12 > x3f.len() {
            break;
        }
        let soff = le32(x3f, e) as usize;
        let slen = le32(x3f, e + 4) as usize;
        if &x3f[e + 8..e + 12] == b"CAMF" && soff + slen <= x3f.len() {
            return decode_camf_type4(&x3f[soff..soff + slen]);
        }
    }
    Err("no CAMF section found in X3F".into())
}

fn cstr_in(buf: &[u8], start: usize, end: usize) -> String {
    if start >= end || end > buf.len() {
        return String::new();
    }
    let s = &buf[start..end];
    let len = s.iter().position(|&b| b == 0).unwrap_or(s.len());
    String::from_utf8_lossy(&s[..len]).into_owned()
}

/// Parses `CMbT`, `CMbP`, and `CMbM` records from a decompressed CAMF buffer.
pub fn parse_camf(buf: &[u8]) -> Vec<CamfEntry> {
    let mut entries = Vec::new();
    let mut pos = 0usize;
    let n = buf.len();
    while pos + 20 <= n {
        let tag = &buf[pos..pos + 4];
        if !matches!(tag, b"CMbM" | b"CMbP" | b"CMbT") {
            break;
        }
        let esz = le32(buf, pos + 8) as usize;
        let noff = le32(buf, pos + 12) as usize;
        let voff = le32(buf, pos + 16) as usize;
        if esz == 0 || pos + esz > n {
            break;
        }
        let end = pos + esz;
        let name = {
            let s = cstr_in(buf, pos + noff, end);
            if s.is_empty() { format!("unk_{pos:x}") } else { s }
        };
        let value = match tag {
            b"CMbT" if pos + voff + 4 <= end => {
                let tlen = le32(buf, pos + voff) as usize;
                let s0 = pos + voff + 4;
                let s1 = (s0 + tlen).min(end);
                CamfValue::Text(cstr_in(buf, s0, s1))
            }
            b"CMbP" if pos + voff + 8 <= end => {
                let cnt = le32(buf, pos + voff) as usize;
                let doff = le32(buf, pos + voff + 4) as usize;
                let mut props = Vec::with_capacity(cnt);
                for i in 0..cnt {
                    let p = pos + voff + 8 + i * 8;
                    if p + 8 > end {
                        break;
                    }
                    let ko = le32(buf, p) as usize;
                    let vo = le32(buf, p + 4) as usize;
                    props.push((cstr_in(buf, pos + doff + ko, end), cstr_in(buf, pos + doff + vo, end)));
                }
                CamfValue::Props(props)
            }
            b"CMbM" if pos + voff + 12 <= end => {
                let dtype = le32(buf, pos + voff);
                let ndim = le32(buf, pos + voff + 4) as usize;
                let doff = le32(buf, pos + voff + 8) as usize;
                let mut dims = Vec::with_capacity(ndim);
                let mut cnt = 1usize;
                for d in 0..ndim {
                    let p = pos + voff + 12 + d * 12;
                    if p + 4 > end {
                        break;
                    }
                    let sz = le32(buf, p);
                    dims.push(sz);
                    cnt = cnt.saturating_mul(sz as usize);
                }
                let dpos = pos + doff;
                let vals = if cnt <= 256 {
                    match dtype {
                        0 if dpos + cnt * 2 <= end => MatrixVals::I16(
                            (0..cnt).map(|i| i16::from_le_bytes([buf[dpos + i * 2], buf[dpos + i * 2 + 1]])).collect(),
                        ),
                        1 if dpos + cnt * 4 <= end => MatrixVals::I32((0..cnt).map(|i| le32(buf, dpos + i * 4) as i32).collect()),
                        2 if dpos + cnt * 4 <= end => MatrixVals::U32((0..cnt).map(|i| le32(buf, dpos + i * 4)).collect()),
                        3 if dpos + cnt * 4 <= end => MatrixVals::F32(
                            (0..cnt).map(|i| f32::from_le_bytes(le32(buf, dpos + i * 4).to_le_bytes())).collect(),
                        ),
                        5 if dpos + cnt <= end => MatrixVals::U8(buf[dpos..dpos + cnt].to_vec()),
                        6 if dpos + cnt * 2 <= end => MatrixVals::U16(
                            (0..cnt).map(|i| u16::from_le_bytes([buf[dpos + i * 2], buf[dpos + i * 2 + 1]])).collect(),
                        ),
                        _ => MatrixVals::Large(cnt),
                    }
                } else {
                    MatrixVals::Large(cnt)
                };
                CamfValue::Matrix { dtype, dims, vals }
            }
            _ => {
                pos += esz;
                continue;
            }
        };
        entries.push(CamfEntry { name, value });
        pos += esz;
    }
    entries
}

pub fn format_entry(e: &CamfEntry) -> String {
    match &e.value {
        CamfValue::Text(s) => format!("[T] {:<34}: {s}", e.name),
        CamfValue::Props(ps) => {
            let body: Vec<String> = ps.iter().map(|(k, v)| format!("{k}={v}")).collect();
            format!("[P] {:<34}: {{{}}}", e.name, body.join(", "))
        }
        CamfValue::Matrix { dtype, dims, vals } => {
            let vstr = match vals {
                MatrixVals::I16(v) => format!("{v:?}"),
                MatrixVals::I32(v) => format!("{v:?}"),
                MatrixVals::U32(v) => format!("{v:?}"),
                MatrixVals::F32(v) => {
                    let s: Vec<String> = v.iter().map(|x| format!("{x:.6}")).collect();
                    format!("[{}]", s.join(", "))
                }
                MatrixVals::U8(v) => format!("{v:?}"),
                MatrixVals::U16(v) => format!("{v:?}"),
                MatrixVals::Large(n) => format!("<{n} elems>"),
            };
            format!("[M] {:<34}: dtype={dtype} dims={dims:?} {vstr}", e.name)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_cmbt_cmbp_cmbm() {
        let mut buf = Vec::new();
        // CMbT: name="Model", val="DP2 Merrill"
        let mut rec = Vec::new();
        rec.extend_from_slice(b"CMbT");
        rec.extend_from_slice(&0x0002_0000u32.to_le_bytes());
        rec.extend_from_slice(&44u32.to_le_bytes()); // esz
        rec.extend_from_slice(&20u32.to_le_bytes()); // noff
        rec.extend_from_slice(&28u32.to_le_bytes()); // voff
        rec.extend_from_slice(b"Model\0\0\0");
        rec.extend_from_slice(&12u32.to_le_bytes());
        rec.extend_from_slice(b"DP2 Merrill\0");
        buf.extend_from_slice(&rec);

        // CMbM: name="CCMatrix", dtype=3 (f32), dims=[2], vals=[1.5, -0.25]
        let mut m = Vec::new();
        m.extend_from_slice(b"CMbM");
        m.extend_from_slice(&0x0002_0000u32.to_le_bytes());
        m.extend_from_slice(&64u32.to_le_bytes());
        m.extend_from_slice(&20u32.to_le_bytes());
        m.extend_from_slice(&32u32.to_le_bytes());
        m.extend_from_slice(b"CCMatrix\0\0\0\0");
        m.extend_from_slice(&3u32.to_le_bytes()); // f32
        m.extend_from_slice(&1u32.to_le_bytes()); // ndim
        m.extend_from_slice(&56u32.to_le_bytes()); // doff
        m.extend_from_slice(&2u32.to_le_bytes()); // dim[0]
        m.extend_from_slice(&0u32.to_le_bytes());
        m.extend_from_slice(&0u32.to_le_bytes());
        m.extend_from_slice(&1.5f32.to_le_bytes());
        m.extend_from_slice(&(-0.25f32).to_le_bytes());
        buf.extend_from_slice(&m);

        let entries = parse_camf(&buf);
        assert_eq!(entries.len(), 2);
        assert_eq!(entries[0].name, "Model");
        assert_eq!(entries[0].value, CamfValue::Text("DP2 Merrill".into()));
        assert_eq!(entries[1].name, "CCMatrix");
        assert_eq!(
            entries[1].value,
            CamfValue::Matrix { dtype: 3, dims: vec![2], vals: MatrixVals::F32(vec![1.5, -0.25]) }
        );
    }
}
