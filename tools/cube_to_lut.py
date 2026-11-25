#!/usr/bin/env python3
"""Converts a standard Adobe/Resolve `.cube` 3D LUT into a 4,160-byte D2ML SD-card binary (`LUT1.BIN`..`LUT8.BIN`).

M5Slave loads `D:\\LUT[1..8].BIN` directly at capture time via `MDF_W_open`/`MDF_W_read`/`MDF_W_close`.
Binary layout (Big-Endian, 0x1040 bytes):
  0x0000 (4 B): magic "D2ML"
  0x0004 (1 B): version = 1
  0x0005 (1 B): grain_amp [0..64]
  0x0006 (1 B): grain_profile (0=Neg, 1=Slide, 2=B&W)
  0x0007 (1 B): halo_gain [0..255]
  0x0008 (1 B): halo_tint (0=Crimson, 1=Neutral B&W, 2=Warm Amber)
  0x0009 (7 B): ASCII tag (zero-padded)
  0x0010 (4,131 B): 17x9x9x3 Q1 (0.5-LSB) signed int8 YCbCr residual 3D LUT
  0x1033 (13 B): zero pad
"""
from __future__ import annotations

import argparse
import struct
from pathlib import Path

import numpy as np
from scipy.sparse import csr_matrix, lil_matrix
from scipy.sparse.linalg import spsolve

NY, NCB, NCR = 17, 9, 9
N_VERT = NY * NCB * NCR


def rgb_to_ycbcr_bt601(rgb01: np.ndarray) -> np.ndarray:
    r, g, b = rgb01[:, 0] * 255.0, rgb01[:, 1] * 255.0, rgb01[:, 2] * 255.0
    y = np.clip(0.299 * r + 0.587 * g + 0.114 * b, 0.0, 255.0)
    cb = np.clip(-0.168736 * r - 0.331264 * g + 0.500 * b + 128.0, 0.0, 255.0)
    cr = np.clip(0.500 * r - 0.418688 * g - 0.081312 * b + 128.0, 0.0, 255.0)
    return np.stack([y, cb, cr], axis=1)


def build_3d_laplacian() -> csr_matrix:
    L = lil_matrix((N_VERT, N_VERT), dtype=np.float64)
    for iy in range(NY):
        for icb in range(NCB):
            for icr in range(NCR):
                u = (iy * NCB + icb) * NCR + icr
                deg = 0
                for dy, dcb, dcr in ((-1, 0, 0), (1, 0, 0), (0, -1, 0), (0, 1, 0), (0, 0, -1), (0, 0, 1)):
                    ny, ncb, ncr = iy + dy, icb + dcb, icr + dcr
                    if 0 <= ny < NY and 0 <= ncb < NCB and 0 <= ncr < NCR:
                        L[u, (ny * NCB + ncb) * NCR + ncr] = -1.0
                        deg += 1
                L[u, u] = float(deg)
    return L.tocsr()


def build_tetra_matrix(ycc_u8: np.ndarray) -> csr_matrix:
    """Sakamoto 6-simplex tetrahedral interpolation design matrix matching M5Slave FR80 layout."""
    n = ycc_u8.shape[0]
    y = np.clip(ycc_u8[:, 0], 0.0, 255.0)
    cb = np.clip(ycc_u8[:, 1], 0.0, 255.0)
    cr = np.clip(ycc_u8[:, 2], 0.0, 255.0)

    iy = np.minimum((y // 16.0).astype(np.int32), NY - 2)
    icb = np.minimum((cb // 32.0).astype(np.int32), NCB - 2)
    icr = np.minimum((cr // 32.0).astype(np.int32), NCR - 2)

    fx = (y - iy * 16.0) / 16.0
    fy = (cb - icb * 32.0) / 32.0
    fz = (cr - icr * 32.0) / 32.0

    sy, scb, scr = NCB * NCR, NCR, 1
    p0 = iy * sy + icb * scb + icr * scr
    p3 = p0 + sy + scb + scr

    p1 = np.empty(n, dtype=np.int32)
    p2 = np.empty(n, dtype=np.int32)
    w0 = np.empty(n, dtype=np.float64)
    w1 = np.empty(n, dtype=np.float64)
    w2 = np.empty(n, dtype=np.float64)
    w3 = np.empty(n, dtype=np.float64)

    m1 = (fx >= fy) & (fy >= fz)
    m2 = (fx >= fz) & (fz > fy)
    m3 = (fz > fx) & (fx >= fy)
    m4 = (fz >= fy) & (fy > fx)
    m5 = (fy > fz) & (fz >= fx)
    m6 = ~(m1 | m2 | m3 | m4 | m5)

    for mask, d1, d2, a, b, c in [
        (m1, sy, sy + scb, fx, fy, fz),
        (m2, sy, sy + scr, fx, fz, fy),
        (m3, scr, sy + scr, fz, fx, fy),
        (m4, scr, scb + scr, fz, fy, fx),
        (m5, scb, scb + scr, fy, fz, fx),
        (m6, scb, sy + scb, fy, fx, fz),
    ]:
        p1[mask] = p0[mask] + d1
        p2[mask] = p0[mask] + d2
        w0[mask] = 1.0 - a[mask]
        w1[mask] = a[mask] - b[mask]
        w2[mask] = b[mask] - c[mask]
        w3[mask] = c[mask]

    rows = np.repeat(np.arange(n, dtype=np.int32), 4)
    cols = np.column_stack([p0, p1, p2, p3]).ravel()
    vals = np.column_stack([w0, w1, w2, w3]).ravel()
    return csr_matrix((vals, (rows, cols)), shape=(n, N_VERT))


def parse_cube(path: Path) -> np.ndarray:
    size = None
    rows = []
    for raw in path.read_text().splitlines():
        line = raw.strip()
        if not line or line.startswith("#") or line.startswith("TITLE") or line.startswith("DOMAIN_"):
            continue
        if line.startswith("LUT_3D_SIZE"):
            size = int(line.split()[1])
            continue
        parts = line.split()
        if len(parts) == 3:
            rows.append([float(parts[0]), float(parts[1]), float(parts[2])])
    if size is None or len(rows) != size ** 3:
        raise ValueError(f"Invalid .cube LUT: size={size}, rows={len(rows)}")
    return np.array(rows, dtype=np.float64).reshape(size, size, size, 3).transpose(2, 1, 0, 3)


def eval_cube_trilinear(rgb01: np.ndarray, cube: np.ndarray) -> np.ndarray:
    s = cube.shape[0]
    p = np.clip(rgb01, 0.0, 1.0) * (s - 1)
    i0 = np.minimum(np.floor(p).astype(np.int32), s - 2)
    f = p - i0
    r0, g0, b0 = i0[:, 0], i0[:, 1], i0[:, 2]
    fr, fg, fb = f[:, 0:1], f[:, 1:2], f[:, 2:3]
    c00 = cube[r0, g0, b0] * (1.0 - fr) + cube[r0 + 1, g0, b0] * fr
    c01 = cube[r0, g0, b0 + 1] * (1.0 - fr) + cube[r0 + 1, g0, b0 + 1] * fr
    c10 = cube[r0, g0 + 1, b0] * (1.0 - fr) + cube[r0 + 1, g0 + 1, b0] * fr
    c11 = cube[r0, g0 + 1, b0 + 1] * (1.0 - fr) + cube[r0 + 1, g0 + 1, b0 + 1] * fr
    c0 = c00 * (1.0 - fg) + c10 * fg
    c1 = c01 * (1.0 - fg) + c11 * fg
    return np.clip(c0 * (1.0 - fb) + c1 * fb, 0.0, 1.0)


def compile_to_d2ml(
    cube: np.ndarray,
    out_path: Path,
    grain_amp: int,
    grain_profile: int,
    halo_gain: int,
    halo_tint: int,
    tag: str,
) -> None:
    g = np.linspace(0.0, 1.0, 13)
    R, G, B = np.meshgrid(g, g, g, indexing="ij")
    rgb_in = np.stack([R.ravel(), G.ravel(), B.ravel()], axis=1)
    ycc_in = rgb_to_ycbcr_bt601(rgb_in)
    ycc_out = rgb_to_ycbcr_bt601(eval_cube_trilinear(rgb_in, cube))

    A = build_tetra_matrix(ycc_in)
    L = build_3d_laplacian()
    H = (A.T @ A + 0.05 * (L.T @ L)).tocsc()
    lut_f = np.column_stack([spsolve(H, A.T @ (ycc_out[:, ch] - ycc_in[:, ch])) for ch in range(3)])
    lut_i8 = np.clip(np.rint(lut_f * 2.0), -126, 126).astype(np.int8)

    pred = np.clip(ycc_in + A @ (lut_i8.astype(np.float64) * 0.5), 0.0, 255.0)
    mae = float(np.abs(pred - ycc_out).mean())

    hdr = struct.pack(
        ">4sBBBBB7s",
        b"D2ML", 1,
        grain_amp & 0xFF, grain_profile & 0xFF, halo_gain & 0xFF, halo_tint & 0xFF,
        tag.encode("ascii", "ignore")[:7].ljust(7, b"\x00"),
    )
    blob = hdr + lut_i8.tobytes()
    blob += bytes(0x1040 - len(blob))
    out_path.write_bytes(blob)
    print(f"Wrote {out_path} ({len(blob)} B), mean YCbCr error = {mae:.2f} LSB")


def main() -> None:
    ap = argparse.ArgumentParser(description="Compile .cube 3D LUT to DP2M D2ML SD-card binary")
    ap.add_argument("input", type=Path, help="Input .cube file")
    ap.add_argument("output", type=Path, help="Output path (e.g. LUT1.BIN)")
    ap.add_argument("--grain", type=int, default=28)
    ap.add_argument("--grain-profile", type=int, default=0, help="0=Neg, 1=Slide, 2=B&W")
    ap.add_argument("--halo", type=int, default=64)
    ap.add_argument("--halo-tint", type=int, default=0, help="0=Crimson, 1=Neutral B&W, 2=Amber")
    args = ap.parse_args()

    cube = parse_cube(args.input)
    compile_to_d2ml(cube, args.output, args.grain, args.grain_profile, args.halo, args.halo_tint, args.input.stem[:7])


if __name__ == "__main__":
    main()
