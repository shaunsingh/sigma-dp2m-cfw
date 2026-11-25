#!/usr/bin/env python3
"""Packs compiled spectral film, B&W, and Quattro calibration JSONs into `data/film_pack.bin`

Run this only when updating offline film physics data. The Rust `dp2m` patcher reads
`data/film_pack.bin` directly with zero Python or JSON dependencies at build time
"""
from __future__ import annotations

import argparse
import hashlib
import json
import struct
import sys
from pathlib import Path

import numpy as np

BW_STOCK_KEYS = [
    "bw_trix_400",
    "bw_hp5_plus",
    "bw_acros_100",
    "bw_tmax_100",
    "bw_retro_80s",
]

BW_PAPER_KEYS = [
    "neutral_fiber",
    "selenium_tone",
    "warmtone_fiber",
    "platinum_cool",
    "grade4_punch",
    "grade1_soft",
    "split_sepia",
    "lith_print",
]


def pack_i16_be(arr: np.ndarray) -> bytes:
    return struct.pack(f">{arr.size}h", *[int(v) for v in arr.ravel()])


def pack_i8(vals: list[int]) -> bytes:
    return bytes(int(v) & 0xFF for v in vals)


def main() -> None:
    ap = argparse.ArgumentParser(description="Build deterministic D2MP v1 data/film_pack.bin")
    ap.add_argument("--dp2q-camf", type=Path, required=True)
    ap.add_argument("--dp2q-to-mcas", type=Path, required=True)
    ap.add_argument("--v3-pack", type=Path, required=True)
    ap.add_argument("--v5-pack", type=Path, required=True)
    ap.add_argument("--out", type=Path, default=Path("data/film_pack.bin"))
    args = ap.parse_args()

    dp2q_camf = json.loads(args.dp2q_camf.read_text())
    dp2q_to = np.array(json.loads(args.dp2q_to_mcas.read_text())["mck_q10"], dtype=np.int16)
    v3_pack = json.loads(args.v3_pack.read_text())
    v5_pack = json.loads(args.v5_pack.read_text())

    recipes = v3_pack["recipes"]
    c3d = v5_pack["color_3dlut"]
    bw_recipes = v5_pack["bw_pack"]["recipes"]
    rom_tbl = v5_pack["rom_tables"]
    papers = v3_pack["papers"]
    rev_films = v3_pack["reversal_films"]
    neg_films = v3_pack["negative_films"]

    color_order = [f"{fk}__reversal" for fk in rev_films]
    for nf in neg_films:
        for pk in papers:
            color_order.append(f"{nf['film']}__{pk}")

    assert len(rev_films) == 4
    assert len(neg_films) == 16
    assert len(papers) == 8
    assert len(color_order) == 132

    buf = bytearray()
    buf += struct.pack(">4s7I", b"D2MP", 1, 132, 40, 16, 4, 5, 0)

    # 1. Official Quattro CMCM matrices (4 x 18 B)
    for key in ("CMCM_FCYellow", "CMCM_Cinema", "CMCM_SunsetRed", "CMCM_ForestGreen"):
        m = np.rint(np.array(dp2q_camf[key]["vals"], dtype=np.float64).reshape(3, 3) * 1024.0).astype(np.int16)
        buf += pack_i16_be(m)

    # 2. Official Quattro Teal & Orange 12-sector MCAS (12 x 18 B)
    assert dp2q_to.shape == (12, 3, 3)
    buf += pack_i16_be(dp2q_to)

    # 3. Default paper indices for the 16 negative films (16 B)
    buf += bytes(papers.index(nf["default_paper"]) for nf in neg_films)

    # 4. Linear-light halation excess LUT (256 B) + 3 density-space grain envelopes (768 B)
    halo_lut = bytes(rom_tbl["halo_linear_excess_u8"])
    grain_envs = (
        bytes(rom_tbl["grain_env_neg_u8"])
        + bytes(rom_tbl["grain_env_slide_u8"])
        + bytes(rom_tbl["grain_env_bw_u8"])
    )
    assert len(halo_lut) == 256 and len(grain_envs) == 768
    buf += halo_lut
    buf += grain_envs

    # 5. 132 Color film recipes (4 reversal + 16x8 negative x paper)
    for pkey in color_order:
        r = recipes[pkey]
        r5 = c3d[pkey]
        # fujifilm_xtra_400__kodak_2393 had a 4.33x blue-row least-squares outlier;
        # substitute its clean sibling cinema-print base (kodak_2383) while keeping its own 3D LUT.
        r_base = recipes["fujifilm_xtra_400__kodak_2383"] if pkey == "fujifilm_xtra_400__kodak_2393" else r
        cmcm = np.array(r_base["cmcm_q10"], dtype=np.int16)
        mck_s = np.array(r["mck_srgb_q10"], dtype=np.int16)
        mck_a = np.array(r["mck_argb_q10"], dtype=np.int16)
        lut_s = bytes(r_base["lut_srgb_u8"])
        lut_a = bytes(r_base["lut_argb_u8"])
        l3_s = pack_i8(r5["lut_srgb_q1_i8"])
        l3_a = pack_i8(r5["lut_argb_q1_i8"])
        meta = bytes([
            int(r5["grain_amp"]) & 0xFF,
            int(r5["grain_profile"]) & 0xFF,
            int(r5["halo_gain"]) & 0xFF,
            int(r5["halo_tint"]) & 0xFF,
        ])
        assert cmcm.shape == (3, 3) and mck_s.shape == (12, 3, 3) and mck_a.shape == (12, 3, 3)
        assert len(lut_s) == 4096 and len(lut_a) == 4096 and len(l3_s) == 4131 and len(l3_a) == 4131
        buf += pack_i16_be(cmcm)
        buf += pack_i16_be(mck_s)
        buf += pack_i16_be(mck_a)
        buf += lut_s
        buf += lut_a
        buf += l3_s
        buf += l3_a
        buf += meta

    # 6. 20 Color Live View luminance 4096-entry curves + raw CMCM (4 reversal + 16 default-paper neg)
    lv_keys = [f"{fk}__reversal" for fk in rev_films] + [
        f"{nf['film']}__{nf['default_paper']}" for nf in neg_films
    ]
    assert len(lv_keys) == 20
    for pkey in lv_keys:
        luts3 = [np.array(ch, dtype=np.float64) for ch in recipes[pkey]["lv_luts_3_u8"]]
        lum = np.clip(np.rint(0.299 * luts3[0] + 0.587 * luts3[1] + 0.114 * luts3[2]), 0, 255).astype(np.uint8)
        assert len(lum) == 4096
        buf += lum.tobytes()
    for pkey in lv_keys:
        cmcm = np.array(recipes[pkey]["cmcm_q10"], dtype=np.int16)
        buf += pack_i16_be(cmcm)

    # 7. 40 B&W silver-halide recipes (5 stocks x 8 darkroom papers/toners)
    bw_count = 0
    for sk in BW_STOCK_KEYS:
        for pk in BW_PAPER_KEYS:
            r = bw_recipes[f"{sk}__{pk}"]
            cmcm = np.array(r["cmcm_q10"], dtype=np.int16)
            lut_s = bytes(r["lut_srgb_u8"])
            lut_a = bytes(r["lut_argb_u8"])
            toner = pack_i8(r["toner_cb_cr_i8"])
            meta = bytes([
                int(r["grain_amp"]) & 0xFF,
                int(r["grain_profile"]) & 0xFF,
                int(r["halo_gain"]) & 0xFF,
                int(r["halo_tint"]) & 0xFF,
            ])
            assert cmcm.shape == (3, 3) and len(lut_s) == 4096 and len(lut_a) == 4096 and len(toner) == 512
            buf += pack_i16_be(cmcm)
            buf += lut_s
            buf += lut_a
            buf += toner
            buf += meta
            bw_count += 1
    assert bw_count == 40

    args.out.parent.mkdir(parents=True, exist_ok=True)
    args.out.write_bytes(buf)
    h = hashlib.sha256(buf).hexdigest()[:16]
    print(f"Wrote {args.out}: {len(buf):,} bytes, sha256={h}")


if __name__ == "__main__":
    main()
