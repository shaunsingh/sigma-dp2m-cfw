"""Capstone big-endian ARM/Thumb disassembly of S3 ranges with literal, call & string annotation."""

from __future__ import annotations

import re
from collections.abc import Iterator, Mapping
from dataclasses import dataclass

from capstone import CS_ARCH_ARM, CS_MODE_ARM, CS_MODE_BIG_ENDIAN, CS_MODE_THUMB, Cs

from .image import END, Image

_CS = {False: Cs(CS_ARCH_ARM, CS_MODE_THUMB | CS_MODE_BIG_ENDIAN), True: Cs(CS_ARCH_ARM, CS_MODE_ARM | CS_MODE_BIG_ENDIAN)}
_PC_LIT = re.compile(r"\[pc, #(-?(?:0x[0-9a-f]+|\d+))\]")
_PC_ADD = re.compile(r"^r\d+, pc, #(0x[0-9a-f]+|\d+)$")
_TARGET = re.compile(r"^#(0x[0-9a-f]+)$")
CHUNK = 0x1000


@dataclass(frozen=True)
class Insn:
    va: int
    raw: bytes
    mnemonic: str
    op_str: str
    note: str = ""

    def __str__(self) -> str:
        s = f"{self.va:08x}: {self.raw.hex():<8} {self.mnemonic:<7} {self.op_str}"
        return f"{s}  ; {self.note}" if self.note else s


def insns(img: Image, va: int, end: int, arm: bool = False) -> Iterator[Insn]:
    """Decode [va, end); undecodable units become `.hword` / `.word`."""
    cs, unit = _CS[arm], 4 if arm else 2
    while va < end:
        stop = min(end, va + CHUNK)
        nxt = va
        # +2: a Thumb BL pair may straddle the chunk end
        for a, size, mn, ops in cs.disasm_lite(img.rd(va, min(stop + 2, END) - va), va):
            if a >= stop:
                break
            yield Insn(a, img.rd(a, size), mn, ops)
            nxt = a + size
        if nxt >= stop:
            va = nxt
            continue
        raw = img.rd(nxt, unit)
        yield Insn(nxt, raw, ".word" if arm else ".hword", f"{int.from_bytes(raw, 'big'):#x}")
        va = nxt + unit


class Annotator:
    """Notes for literal loads, pc-relative addresses and branch targets."""

    def __init__(self, img: Image, names: Mapping[int, str] | None = None):
        self.img, self.names = img, names or {}

    def value(self, v: int) -> str:
        name = self.names.get(v & ~1)
        if name:
            return name
        s = self.img.text_at(v)
        return f'"{s[:48]}"' if s else ""

    def note(self, i: Insn, arm: bool) -> str:
        pc = i.va + 8 if arm else (i.va + 4) & ~3
        if i.mnemonic.startswith("ldr") and (m := _PC_LIT.search(i.op_str)):
            lit = pc + int(m.group(1), 0)
            try:
                v = self.img.u32(lit)
            except IndexError:
                return ""
            return f"={v:#010x} {self.value(v)}".rstrip()
        if i.mnemonic == "adr" and (m := _TARGET.search(i.op_str.split(", ")[-1])):
            return self.value(pc + int(m.group(1), 0)) or f"{pc + int(m.group(1), 0):#010x}"
        if i.mnemonic in ("add", "sub") and arm and (m := _PC_ADD.match(i.op_str)):
            t = pc + int(m.group(1), 0) * (1 if i.mnemonic == "add" else -1)
            return f"{t:#010x} {self.value(t)}".rstrip()
        if i.mnemonic.startswith("b") and (m := _TARGET.match(i.op_str)):
            return self.names.get(int(m.group(1), 16) & ~1, "")
        return ""

    def lines(self, va: int, end: int, arm: bool = False) -> Iterator[str]:
        for i in insns(self.img, va, end, arm):
            label = self.names.get(i.va)
            if label:
                yield f"{label}:"
            yield str(Insn(i.va, i.raw, i.mnemonic, i.op_str, self.note(i, arm)))
