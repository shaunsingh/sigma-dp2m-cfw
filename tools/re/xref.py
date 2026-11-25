"""One-pass reference index of an S3 image (calls, branches, literal loads, adr, data pointers), cached per image."""

from __future__ import annotations

import hashlib
import os
from dataclasses import dataclass
from pathlib import Path

import numpy as np

from .image import BASE, CODE_END, FPGA, SDRAM_END, Image

KINDS = ("tbl", "tblx", "tb", "tbc", "tlit", "tadr", "abl", "ablx", "ab", "alit", "aadr", "w", "w2")
CALLS = ("tbl", "tblx", "abl", "ablx")
FORMAT = 1


@dataclass(frozen=True)
class Ref:
    src: int  # referencing instruction or data word
    val: int  # branch target, loaded literal value, adr target or pointer value
    slot: int  # literal slot of loads, else src
    kind: str


def cache_dir() -> Path:
    d = Path(os.environ.get("XDG_CACHE_HOME") or Path.home() / ".cache") / "dp2m-re"
    d.mkdir(parents=True, exist_ok=True)
    return d


def _sext(v: np.ndarray, bits: int) -> np.ndarray:
    return v - ((v & (1 << (bits - 1))) << 1)


def _build(s3: bytes) -> tuple[np.ndarray, ...]:
    h = np.frombuffer(s3, ">u2").astype(np.int64)
    w = np.frombuffer(s3, ">u4").astype(np.int64)
    parts: list[tuple[np.ndarray, np.ndarray, np.ndarray, str]] = []

    def add(src: np.ndarray, val: np.ndarray, kind: str, slot: np.ndarray | None = None) -> None:
        parts.append((src, val, src if slot is None else slot, kind))

    def lit(slot: np.ndarray) -> tuple[np.ndarray, np.ndarray]:
        ok = (slot >= BASE) & (slot <= BASE + len(s3) - 4)
        return np.where(ok, w[np.clip((slot - BASE) >> 2, 0, len(w) - 1)], 0), ok

    # Thumb, every halfword of the code region
    th = h[: (CODE_END - BASE) // 2]
    va = BASE + 2 * np.arange(len(th), dtype=np.int64)
    i = np.nonzero(((th[:-1] & 0xF800) == 0xF000) & ((th[1:] & 0xE800) == 0xE800))[0]
    hi, lo = th[i], th[i + 1]
    t = va[i] + 4 + _sext(((hi & 0x7FF) << 12) | ((lo & 0x7FF) << 1), 23)
    blx = (lo & 0xF800) == 0xE800
    add(va[i][~blx], t[~blx], "tbl")
    add(va[i][blx], t[blx] & ~3, "tblx")
    i = np.nonzero((th & 0xF800) == 0xE000)[0]
    add(va[i], va[i] + 4 + _sext((th[i] & 0x7FF) << 1, 12), "tb")
    i = np.nonzero(((th & 0xF000) == 0xD000) & ((th & 0x0F00) < 0x0E00))[0]
    add(va[i], va[i] + 4 + _sext((th[i] & 0xFF) << 1, 9), "tbc")
    for op, kind in ((0x4800, "tlit"), (0xA000, "tadr")):
        i = np.nonzero((th & 0xF800) == op)[0]
        slot = ((va[i] + 4) & ~3) + (th[i] & 0xFF) * 4
        if kind == "tadr":
            add(va[i], slot, kind)
        else:
            v, ok = lit(slot)
            add(va[i][ok], v[ok], kind, slot[ok])

    # ARM, every word of the code region
    aw = w[: (CODE_END - BASE) // 4]
    va = BASE + 4 * np.arange(len(aw), dtype=np.int64)
    i = np.nonzero((aw & 0x0E000000) == 0x0A000000)[0]
    x = aw[i]
    t = va[i] + 8 + _sext((x & 0xFFFFFF) << 2, 26)
    uncond = (x >> 28) == 0xF
    link = (x & 0x01000000) != 0
    add(va[i][uncond], (t + ((x >> 23) & 2))[uncond], "ablx")
    add(va[i][~uncond & link], t[~uncond & link], "abl")
    add(va[i][~uncond & ~link], t[~uncond & ~link], "ab")
    i = np.nonzero((aw & 0x0FEF0000) == 0x028F0000)[0]
    x = aw[i]
    rot = ((x >> 8) & 0xF) * 2
    imm = (((x & 0xFF) >> rot) | ((x & 0xFF) << (32 - rot))) & 0xFFFFFFFF
    add(va[i], np.where(((x >> 21) & 0xF) == 4, va[i] + 8 + imm, va[i] + 8 - imm), "aadr")
    i = np.nonzero((aw & 0x0F3F0000) == 0x051F0000)[0]
    x = aw[i]
    slot = va[i] + 8 + np.where(x & 0x00800000, x & 0xFFF, -(x & 0xFFF))
    v, ok = lit(slot)
    add(va[i][ok], v[ok], "alit", slot[ok])

    # SDRAM pointers in data: aligned words anywhere, halfword-aligned words outside code; not the bitstream
    va = BASE + 4 * np.arange(len(w), dtype=np.int64)
    keep = (w >= BASE) & (w < SDRAM_END) & ~((va >= FPGA[0]) & (va < FPGA[1]))
    add(va[keep], w[keep], "w")
    u = (h[1:-1:2] << 16) | h[2::2]
    va = BASE + 2 + 4 * np.arange(len(u), dtype=np.int64)
    keep = (u >= BASE) & (u < SDRAM_END) & (va >= FPGA[1])
    add(va[keep], u[keep], "w2")

    src = np.concatenate([p[0] for p in parts]).astype(np.uint32)
    val = (np.concatenate([p[1] for p in parts]) & 0xFFFFFFFF).astype(np.uint32)
    slot = np.concatenate([p[2] for p in parts]).astype(np.uint32)
    kind = np.concatenate([np.full(len(p[0]), KINDS.index(p[3]), np.uint8) for p in parts])
    o = np.argsort(val, kind="stable")
    return src[o], val[o], slot[o], kind[o]


class Index:
    """References sorted by value; `refs(v)` matches v with either Thumb bit."""

    def __init__(self, src: np.ndarray, val: np.ndarray, slot: np.ndarray, kind: np.ndarray):
        self.src, self.val, self.slot, self.kind = src, val, slot, kind

    @classmethod
    def of(cls, img: Image) -> Index:
        key = hashlib.blake2b(img.s3, digest_size=10).hexdigest()
        path = cache_dir() / f"xref{FORMAT}-{key}.npz"
        if path.exists():
            with np.load(path) as z:
                return cls(z["src"], z["val"], z["slot"], z["kind"])
        arrays = _build(img.s3)
        tmp = path.with_suffix(f".{os.getpid()}.npz")
        np.savez(tmp, **dict(zip(("src", "val", "slot", "kind"), arrays)))
        tmp.replace(path)
        return cls(*arrays)

    def __len__(self) -> int:
        return len(self.val)

    def refs(self, value: int, kinds: tuple[str, ...] | None = None) -> list[Ref]:
        """References to value (either Thumb bit), plus literal loads whose slot is value."""
        ks = list(np.nonzero((self.slot == value) & (self.slot != self.src))[0])
        for v in sorted({value & ~1, value | 1}):
            lo, hi = np.searchsorted(self.val, [v, v + 1])
            ks += range(lo, hi)
        out = []
        for k in ks:
            kind = KINDS[self.kind[k]]
            if kinds is None or kind in kinds:
                out.append(Ref(int(self.src[k]), int(self.val[k]), int(self.slot[k]), kind))
        return sorted(set(out), key=lambda r: r.src)

    def callers(self, target: int) -> list[int]:
        return [r.src for r in self.refs(target, CALLS)]


def fn_start(img: Image, va: int, back: int = 0x1000) -> int | None:
    """Nearest preceding Thumb `push {.., lr}`."""
    for a in range(va & ~1, max(BASE, va - back), -2):
        if img.u16(a) & 0xFF00 == 0xB500:
            return a
    return None
