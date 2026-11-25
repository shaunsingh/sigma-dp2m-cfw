"""Static NVRAM coverage: register-tracking scan of Thumb code for accesses into a RAM window."""

from __future__ import annotations

from collections import defaultdict
from collections.abc import Iterator
from dataclasses import dataclass

import numpy as np
from capstone import CS_ARCH_ARM, CS_MODE_BIG_ENDIAN, CS_MODE_THUMB, Cs
from capstone.arm import ARM_OP_IMM, ARM_OP_MEM, ARM_OP_REG, ARM_REG_PC, ARM_REG_SP

from .image import BASE, CODE_END, Image
from .xref import Index

_CS = Cs(CS_ARCH_ARM, CS_MODE_THUMB | CS_MODE_BIG_ENDIAN)
_CS.detail = True
_R = {_CS.reg_name(i): i for i in range(1, 120) if _CS.reg_name(i)}
SEED_SLACK = 0x100  # seed literals may point just outside the window (negative offsets)


@dataclass(frozen=True)
class Access:
    addr: int
    size: int
    kind: str  # mnemonic, "[idx]" when register-indexed
    va: int
    seed: int


@dataclass(frozen=True)
class Call:
    va: int
    target: int | None
    args: tuple[int | None, int | None, int | None]
    seed: int


def _track(img: Image, seed: int, reg: int, val: int, lo: int, hi: int, maxn: int, acc: list, calls: list) -> None:
    """Follow one literal-seeded register linearly (no branches), constant-folding adds/shifts/moves."""
    regs: dict[int, int | None] = {reg: val}

    def win(v: int | None) -> bool:
        return v is not None and lo <= v < hi

    va = seed + 2
    for _ in range(maxn):
        if va >= CODE_END or not any(win(v) for v in regs.values()):
            return
        ins = next(_CS.disasm(img.rd(va, 4), va, 1), None)
        if ins is None:
            return
        m, ops, nxt = ins.mnemonic, ins.operands, va + ins.size
        if m in ("bl", "blx"):
            calls.append(Call(va, img.bl_target(va), tuple(regs.get(_R[f"r{i}"]) for i in range(3)), seed))
            for r in ("r0", "r1", "r2", "r3", "ip", "lr"):
                regs.pop(_R[r], None)
        elif m in ("b", "bx") or (m == "pop" and any(o.type == ARM_OP_REG and o.reg == ARM_REG_PC for o in ops)):
            return
        elif m.startswith("b") and len(m) <= 4 and m not in ("bic", "bics"):
            pass  # conditional branch: fall through
        elif m.startswith(("ldr", "str")) and len(ops) == 2 and ops[1].type == ARM_OP_MEM:
            mem, dst = ops[1].mem, ops[0].reg
            sz = 1 if m.endswith(("b", "sb")) else 2 if m.endswith(("h", "sh")) else 4
            base = regs.get(mem.base)
            if mem.base == ARM_REG_PC:
                regs.pop(dst, None)
                if sz == 4:
                    regs[dst] = img.u32(((va + 4) & ~3) + mem.disp)
            else:
                if win(base):
                    acc.append(Access(base, sz, m + "[idx]", va, seed) if mem.index else Access(base + mem.disp, sz, m, va, seed))
                if m.startswith("ldr"):
                    regs.pop(dst, None)
        elif m in ("ldm", "ldmia", "stm", "stmia") and ops and ops[0].type == ARM_OP_REG:
            base = regs.get(ops[0].reg)
            if win(base):
                acc.extend(Access(base + 4 * k, 4, m, va, seed) for k in range(len(ops) - 1))
            if m.startswith("ldm"):
                for o in ops[1:]:
                    regs.pop(o.reg, None)
            if ins.writeback and regs.get(ops[0].reg) is not None:
                regs[ops[0].reg] += 4 * (len(ops) - 1)
        elif m in ("cmp", "cmn", "tst"):
            pass
        elif ops and ops[0].type == ARM_OP_REG:
            d, src = ops[0].reg, ops[1:]
            v = None
            if m in ("movs", "mov") and len(src) == 1:
                v = src[0].imm if src[0].type == ARM_OP_IMM else regs.get(src[0].reg)
            elif m in ("adds", "add", "subs", "sub") and src:
                sgn = 1 if m.startswith("add") else -1
                a = regs.get(d) if len(src) == 1 else None if src[0].type != ARM_OP_REG or src[0].reg == ARM_REG_PC else regs.get(src[0].reg)
                b = src[-1].imm if src[-1].type == ARM_OP_IMM else regs.get(src[-1].reg)
                v = None if a is None or b is None or d == ARM_REG_SP else (a + sgn * b) & 0xFFFFFFFF
            elif m in ("lsls", "lsl") and len(src) == 2 and src[1].type == ARM_OP_IMM:
                a = regs.get(src[0].reg)
                v = None if a is None else (a << src[1].imm) & 0xFFFFFFFF
            elif m == "muls" and src:
                a, b = regs.get(d), regs.get(src[-1].reg) if src[-1].type == ARM_OP_REG else None
                v = None if a is None or b is None else (a * b) & 0xFFFFFFFF
            if v is None:
                regs.pop(d, None)
            else:
                regs[d] = v
        va = nxt


def scan(img: Image, lo: int, hi: int, idx: Index | None = None, maxn: int = 48) -> tuple[list[Access], list[Call]]:
    """Accesses into [lo, hi) and calls taking window pointers, from every `ldr rX, =lit` near the window."""
    idx = idx or Index.of(img)
    a, b = np.searchsorted(idx.val, [max(0, lo - SEED_SLACK), hi + SEED_SLACK])
    acc: list[Access] = []
    calls: list[Call] = []
    for k in sorted(range(a, b), key=lambda k: int(idx.src[k])):
        if idx.kind[k] == 4 and BASE <= idx.src[k] < CODE_END:  # tlit
            src = int(idx.src[k])
            _track(img, src, _R[f"r{(img.u16(src) >> 8) & 7}"], int(idx.val[k]), lo, hi, maxn, acc, calls)
    return acc, calls


def _ranges(pred, lo: int, hi: int) -> Iterator[tuple[int, int]]:
    start = None
    for o in range(lo, hi + 1):
        if o < hi and pred(o):
            start = o if start is None else start
        elif start is not None:
            yield start, o
            start = None


def report(img: Image, acc: list[Access], calls: list[Call], block: int, n: int, defaults: int | None = None,
           offsets: tuple[int, ...] = ()) -> Iterator[str]:
    """Calls with window arguments, unreferenced offsets of the block, accesses outside it, per-offset users."""
    cov: defaultdict[int, list[tuple[str, int]]] = defaultdict(list)
    for x in acc:
        for i in range(x.size):
            cov[x.addr + i].append((x.kind, x.va))
    yield "calls with window arguments:"
    for c in calls:
        a = ["?" if v is None else f"{v:#x}" for v in (c.target, *c.args)]
        yield f"  {c.va:#x} -> {a[0]} r0={a[1]} r1={a[2]} r2={a[3]}"
    yield f"block offsets without static field access (+0..+{n:#x}):"
    for s, e in _ranges(lambda o: not cov.get(block + o), 0, n):
        d = f"  defaults {img.rd(defaults + s, min(e - s, 24)).hex()}" if defaults is not None else ""
        yield f"  +{s:#05x}..+{e:#05x} ({e - s:4d} B) @ {block + s:#x}{d}"
    yield "accesses outside the block:"
    for a in sorted({x.addr for x in acc if not block <= x.addr < block + n}):
        yield f"  {a:#x} by {sorted({hex(v) for _, v in cov[a]})[:6]}"
    for o in offsets:
        yield f"+{o:#x} @ {block + o:#x}: " + " ".join(f"{k}@{v:#x}" for k, v in sorted(set(cov[block + o]), key=lambda x: x[1]))
