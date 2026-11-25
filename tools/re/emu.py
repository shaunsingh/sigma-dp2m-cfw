"""Unicorn ARM926EJ-S big-endian machine: S3 image in SDRAM, Thumb/ARM calls, stubs, offscreen text."""

from __future__ import annotations

import struct
from collections.abc import Callable, Iterator
from contextlib import contextmanager

import numpy as np
from unicorn import UC_ARCH_ARM, UC_HOOK_CODE, UC_HOOK_INTR, UC_HOOK_MEM_WRITE, UC_MODE_ARM, UC_MODE_BIG_ENDIAN, Uc, UcError
from unicorn import arm_const

from . import stock
from .image import BASE, Image

RAM_END = 0xA4000000
PERIPH = (0x60000000, 0x200000)  # reads as 0
RET = 0xA27F0000  # return sentinel; emulation stops before executing it
STACK = (0xA1FF0000, 0xA2001000)
SCRATCH = 0xA2B00000  # arguments passed by reference
MASK = 0xFFFFFFFF
ARGS = tuple(getattr(arm_const, f"UC_ARM_REG_R{i}") for i in range(4))
SP, LR, PC = arm_const.UC_ARM_REG_SP, arm_const.UC_ARM_REG_LR, arm_const.UC_ARM_REG_PC


class EmuError(RuntimeError):
    pass


def _reg(name: str) -> int:
    return getattr(arm_const, f"UC_ARM_REG_{name.upper()}")


def outside(writes: list[tuple[int, int]], *allowed: tuple[int, int]) -> list[int]:
    """Addresses of logged writes not inside any allowed [lo, hi)."""
    return sorted({a for a, n in writes if not any(lo <= a and a + n <= hi for lo, hi in allowed)})


class Machine:
    """Fresh S3 + SDRAM + peripheral window; uITRON calls return E_OK, the language getter returns `lang`."""

    def __init__(self, img: Image, lang: int = 0):
        self.img = img
        self.uc = uc = Uc(UC_ARCH_ARM, UC_MODE_ARM | UC_MODE_BIG_ENDIAN)
        uc.mem_map(BASE, RAM_END - BASE)
        uc.mem_write(BASE, img.s3)
        uc.mem_map(*PERIPH)
        uc.hook_add(UC_HOOK_INTR, lambda *_: None)
        self.stub(stock.UITRON[0], ret=0, end=stock.UITRON[1])
        self.stub(stock.GET_LANG, ret=lang)
        self.wr(stock.FONT_SCALE, struct.pack(">II", 1, 1))  # neutral; screens set their own

    # memory
    def rd(self, va: int, n: int) -> bytes:
        return bytes(self.uc.mem_read(va, n))

    def u8(self, va: int) -> int:
        return self.rd(va, 1)[0]

    def u16(self, va: int) -> int:
        return struct.unpack(">H", self.rd(va, 2))[0]

    def u32(self, va: int) -> int:
        return struct.unpack(">I", self.rd(va, 4))[0]

    def cstr(self, va: int, n: int = 256) -> str:
        return self.rd(va, n).split(b"\0")[0].decode("latin1")

    def wr(self, va: int, data: bytes) -> None:
        self.uc.mem_write(va, data)

    def w8(self, va: int, v: int) -> None:
        self.wr(va, bytes([v & 0xFF]))

    def w16(self, va: int, v: int) -> None:
        self.wr(va, struct.pack(">H", v & 0xFFFF))

    def w32(self, va: int, v: int) -> None:
        self.wr(va, struct.pack(">I", v & MASK))

    def patch(self, va: int, data: bytes) -> None:
        """Write code and drop its translation cache."""
        self.wr(va, data)
        self.uc.ctl_remove_cache(va, va + len(data))

    def load_nv_defaults(self) -> None:
        """NVRAM block from the ROM defaults, as after a reset (boot loads it from NOR)."""
        self.wr(stock.NV_BLOCK, self.img.rd(stock.NV_DEFAULTS, stock.NV_LEN))

    # registers
    def reg(self, name: str) -> int:
        return self.uc.reg_read(_reg(name))

    def set_regs(self, **regs: int) -> None:
        for k, v in regs.items():
            self.uc.reg_write(_reg(k), v & MASK)

    def args(self) -> list[int]:
        return [self.uc.reg_read(r) for r in ARGS]

    def stack_args(self, n: int) -> tuple[int, ...]:
        return struct.unpack(f">{n}I", self.rd(self.uc.reg_read(SP), 4 * n))

    # execution
    def _start(self, begin: int, until: int, count: int, what: str) -> None:
        try:
            self.uc.emu_start(begin, until, count=count)
        except UcError as e:
            raise EmuError(f"{what}: {e} at pc {self.uc.reg_read(PC):#x}") from e
        if (pc := self.uc.reg_read(PC)) != until:
            raise EmuError(f"{what}: stopped at {pc:#x} after {count} insns")

    def call(self, va: int, *args: int, stack: tuple[int, ...] = (), arm: bool = False, count: int = 50_000_000) -> int:
        """Call a Thumb (or ARM) function with r0..r3 and stack words; returns r0."""
        sp = STACK[1]
        if stack:
            self.wr(sp, b"".join(struct.pack(">I", v & MASK) for v in stack))
        self.uc.reg_write(SP, sp)
        self.uc.reg_write(LR, RET | 1)
        for r, v in zip(ARGS, args):
            self.uc.reg_write(r, v & MASK)
        self._start(va if arm else va | 1, RET, count, f"call {va:#x}")
        if self.uc.reg_read(SP) != sp:
            raise EmuError(f"call {va:#x}: sp {self.uc.reg_read(SP):#x} != {sp:#x}")
        return self.uc.reg_read(ARGS[0])

    def run(self, start: int, until: int, count: int = 1_000_000, **regs: int) -> None:
        """Run Thumb code from start to until (mid-function); sp defaults to the stack top."""
        self.uc.reg_write(SP, STACK[1])
        self.set_regs(**regs)
        self._start(start | 1, until, count, f"run {start:#x}..{until:#x}")

    def stub(
        self, va: int, ret: int | None = None, on: Callable[[list[int]], int | None] | None = None, end: int | None = None
    ) -> int:
        """Return to lr on reaching va (or [va, end)) with r0 = on(r0..r3) or ret; r0 kept when both are None."""

        def hook(uc: Uc, *_: object) -> None:
            r = on(self.args()) if on else None
            r = ret if r is None else r
            if r is not None:
                uc.reg_write(ARGS[0], r & MASK)
            uc.reg_write(PC, uc.reg_read(LR))

        return self.uc.hook_add(UC_HOOK_CODE, hook, begin=va, end=(end or va + 1) - 1)

    def watch(self, va: int, fn: Callable[[list[int]], object]) -> int:
        """Call fn(r0..r3) whenever execution reaches va; the function still runs."""
        return self.uc.hook_add(UC_HOOK_CODE, lambda *_: fn(self.args()), begin=va, end=va)

    @contextmanager
    def writes(self, *allowed: tuple[int, int]) -> Iterator[list[tuple[int, int]]]:
        """Log (addr, size) of writes outside the allowed [lo, hi) ranges while active."""
        log: list[tuple[int, int]] = []
        gaps, lo = [], 0
        for a, b in sorted(allowed):
            if a > lo:
                gaps.append((lo, a))
            lo = max(lo, b)
        if lo <= MASK:
            gaps.append((lo, MASK + 1))
        hooks = [
            self.uc.hook_add(UC_HOOK_MEM_WRITE, lambda _uc, _acc, addr, size, *_: log.append((addr, size)), begin=a, end=b - 1)
            for a, b in gaps
        ]
        try:
            yield log
        finally:
            for h in hooks:
                self.uc.hook_del(h)

    # stock services
    def text(self, s: str, px: int, style: int = 0x22, rect: tuple[int, int, int, int] = (0, 600, 600, 100), lang: int = 0) -> np.ndarray:
        """Draw `s` with the stock font engine; returns the coverage line buffer (0 none, 1 solid .. 15).

        rect y >= 480 keeps the OSD untouched; the engine does not clear stale line-buffer rows.
        """
        n = stock.LINE_W * stock.LINE_ROWS
        self.wr(SCRATCH, struct.pack(">4H", *rect))
        self.wr(SCRATCH + 8, s.encode("latin1") + b"\0")
        self.call(stock.TEXT_DRAW, lang, SCRATCH + 8, MASK, px, stack=(5, 0, 1, SCRATCH, style, 0))
        return np.frombuffer(self.rd(stock.LINE_BUF, n), np.uint8).reshape(stock.LINE_ROWS, stock.LINE_W)
