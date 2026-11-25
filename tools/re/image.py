"""DP2M firmware images: updater package, M6M C78FRM image and its S3 executable addressed by VA."""

from __future__ import annotations

import struct
from dataclasses import dataclass, field
from pathlib import Path

PKG_MAGIC = b"SIGMA.CODP2M"
C78_MAGIC = b"FUJITSU!"
C78_OFF, C78_LEN = 0x010A4100, 0x00820000
IPL_LEN = 0x20000
BASE, S3_LEN = 0xA0000000, 0x00800000
END = BASE + S3_LEN
CODE_END = 0xA0179000  # ARM/Thumb code; the Spartan-6 bitstream follows
FPGA = (0xA0179000, 0xA023D35A)
BSS = 0xA07FBC1C  # zero-init RAM starts inside the image tail
SDRAM_END = 0xB0000000

# package layout: header, stock bridge M6M (flashed by the running fw), M5, final M6M at hdr[0x64]
SEGMENTS = {"header": (0, 0x80), "bridge": (0x80, 0x820000), "m5": (0x820080, 0x884080)}
VERSION_FIELDS = (0x10, 0x30, 0x40)
FLAGS = {0: "running fw flashes bridge", 4: "bridge flashes M6M @ hdr[0x64]", 5: "bridge flashes M5"}


@dataclass(frozen=True)
class Package:
    """Updater package `DP2MVxxx.bin`."""

    raw: bytes

    def field(self, off: int, n: int = 16) -> str:
        return self.raw[off : off + n].split(b"\0")[0].decode("latin1")

    @property
    def versions(self) -> tuple[str, ...]:
        return tuple(self.field(o) for o in VERSION_FIELDS)

    @property
    def version(self) -> str:
        return self.versions[0]

    @property
    def date(self) -> str:
        return self.field(0x50, 8)

    @property
    def c78_off(self) -> int:
        return struct.unpack_from("<I", self.raw, 0x64)[0]

    @property
    def flags(self) -> int:
        return self.raw[0x70]

    def segments(self) -> dict[str, tuple[int, int]]:
        return {**SEGMENTS, "m6m": (self.c78_off, C78_LEN)}

    def segment(self, name: str) -> bytes:
        off, n = self.segments()[name]
        return self.raw[off : off + n]


@dataclass(frozen=True)
class Image:
    """M6M `C78FRM.bin` (IPL + S3); accessors address S3 by VA."""

    c78: bytes
    pkg: Package | None = None
    path: Path | None = None
    s3: bytes = field(init=False, repr=False)

    def __post_init__(self) -> None:
        if len(self.c78) != C78_LEN or not self.c78.startswith(C78_MAGIC):
            raise ValueError(f"{self.path}: not a DP2M package or C78FRM.bin")
        object.__setattr__(self, "s3", self.c78[IPL_LEN : IPL_LEN + S3_LEN])

    @classmethod
    def load(cls, path: Path | str) -> Image:
        path = Path(path)
        raw = path.read_bytes()
        if raw.startswith(PKG_MAGIC):
            pkg = Package(raw)
            return cls(pkg.segment("m6m"), pkg, path)
        return cls(raw, None, path)

    def rd(self, va: int, n: int) -> bytes:
        if not (BASE <= va and va + n <= END):
            raise IndexError(f"{va:#x}+{n:#x} outside S3")
        return self.s3[va - BASE : va - BASE + n]

    def u8(self, va: int) -> int:
        return self.rd(va, 1)[0]

    def u16(self, va: int) -> int:
        return struct.unpack(">H", self.rd(va, 2))[0]

    def u32(self, va: int) -> int:
        return struct.unpack(">I", self.rd(va, 4))[0]

    def words(self, va: int, n: int) -> tuple[int, ...]:
        return struct.unpack(f">{n}I", self.rd(va, 4 * n))

    def cstr(self, va: int, n: int = 256) -> str:
        return self.rd(va, min(n, END - va)).split(b"\0")[0].decode("utf-8", "backslashreplace")

    def bl_target(self, va: int) -> int | None:
        """Callee of the Thumb BL/BLX pair at va (BLX targets are word-aligned ARM)."""
        hi, lo = self.u16(va), self.u16(va + 2)
        if hi & 0xF800 != 0xF000 or lo & 0xE800 != 0xE800:
            return None
        off = ((hi & 0x7FF) << 12) | ((lo & 0x7FF) << 1)
        t = va + 4 + off - ((off & 0x400000) << 1)
        return t & ~3 if lo & 0xF800 == 0xE800 else t

    def text_at(self, va: int, min_len: int = 3) -> str | None:
        """Printable C string at va, if any."""
        if not BASE <= va < END:
            return None
        s = self.cstr(va, 80)
        return s if len(s) >= min_len and all(32 <= ord(c) < 127 for c in s) else None

    def find(self, pat: bytes, lo: int = BASE, hi: int = END) -> list[int]:
        out, o = [], lo - BASE
        while (i := self.s3.find(pat, o, hi - BASE)) >= 0:
            out.append(BASE + i)
            o = i + 1
        return out


@dataclass(frozen=True)
class Sym:
    addr: int
    size: int

    @property
    def va(self) -> int:
        return self.addr & ~1


def load_syms(path: Path | str) -> dict[str, Sym]:
    """`m6m.syms` ("addr size name" per line) without ELF mapping symbols."""
    out = {}
    for line in Path(path).read_text().splitlines():
        a, n, name = line.split()
        if not name.startswith("$"):
            out[name] = Sym(int(a, 16), int(n))
    return out
