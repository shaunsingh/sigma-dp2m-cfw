"""Recursive-descent code map, function reachability and dead-region candidates of an S3 image."""

from __future__ import annotations

import bisect
import hashlib
import pickle
import re
import struct
from collections import defaultdict, deque
from collections.abc import Iterable
from dataclasses import dataclass, field

import numpy as np

from .image import BASE, BSS, CODE_END, FPGA, S3_LEN, Image
from .xref import cache_dir

ARM, THUMB, LIT = 1, 2, 3  # CodeMap.mode per byte
ROOTS = ((0xA0000000, "A"), (0xA01657EA, "T"))  # reset, main
VECTORS = (0xA07FBC1C, 0xA07FBC1C + 0x338)  # handler pointers in the image tail
SWITCH8 = (0xE55EC001, 0xE12FFF1C)  # __ARM_common_switch8 words 0 and 5: inline byte table follows the BLX
Range = tuple[int, int]


def _looks_arm(s3: bytes, va: int) -> bool:
    if va % 4 or not BASE <= va < CODE_END - 12:
        return False
    return all(w >> 28 == 0xE for w in struct.unpack_from(">3I", s3, va - BASE))


def _rot_imm(w: int) -> int:
    rot, imm = ((w >> 8) & 0xF) * 2, w & 0xFF
    return ((imm >> rot) | (imm << (32 - rot))) & 0xFFFFFFFF


class CodeMap:
    """Per-byte ARM / Thumb / literal map with function entries, edges and indirect sites."""

    def __init__(self, s3: bytes, black: Iterable[int] = ()):
        self.s3 = s3
        self.mode = bytearray(S3_LEN)
        self.entry: dict[int, tuple[str, str]] = {}  # va -> ("T" | "A", origin)
        self.edges: defaultdict[int, set[tuple[int, str]]] = defaultdict(set)
        self.ind: list[tuple[int, str, int]] = []
        self.root: dict[int, int] = {}  # insn -> descent root that decoded it
        self.black = set(black)
        self._cur = 0
        self._sw: dict[int, bool] = {}

    def u16(self, va: int) -> int:
        o = va - BASE
        return (self.s3[o] << 8) | self.s3[o + 1]

    def u32(self, va: int) -> int:
        return struct.unpack_from(">I", self.s3, va - BASE)[0]

    @staticmethod
    def code(va: int) -> bool:
        return BASE <= va < CODE_END

    def is_switch8(self, t: int) -> bool:
        if t not in self._sw:
            self._sw[t] = self.code(t) and t % 4 == 0 and (self.u32(t), self.u32(t + 20)) == SWITCH8
        return self._sw[t]

    def run(self, items: Iterable[tuple[int, str]], origin: str) -> None:
        for va, m in items:
            if va in self.black:
                continue
            wl: deque[tuple[int, str]] = deque()
            self._cur = va
            self.add_fn(va, m, wl, origin)
            while wl:
                v, mm = wl.popleft()
                (self.walk_t if mm == "T" else self.walk_a)(v, wl)

    def add_fn(self, va: int, m: str, wl: deque, origin: str) -> None:
        if not self.code(va):
            return
        self.entry.setdefault(va, (m, origin))
        wl.append((va, m))

    def mark(self, va: int, n: int, v: int) -> None:
        if v in (ARM, THUMB):
            self.root.setdefault(va, self._cur)
        o = va - BASE
        for i in range(o, min(o + n, S3_LEN)):
            if self.mode[i] == 0 or (v in (ARM, THUMB) and self.mode[i] == LIT):
                self.mode[i] = v

    def lit(self, src: int, la: int) -> int | None:
        if not BASE <= la <= BASE + S3_LEN - 4:
            return None
        self.mark(la, 4, LIT)
        v = self.u32(la)
        self.edges[src].add((la, "litslot"))
        if BASE <= v < BASE + S3_LEN:
            self.edges[src].add((v, "lit"))
        return v

    def walk_t(self, va: int, wl: deque) -> None:
        regs: dict[int, int] = {}
        while self.code(va):
            if self.mode[va - BASE] in (ARM, THUMB):
                return
            hw = self.u16(va)
            if hw & 0xF800 == 0xF000:
                lo = self.u16(va + 2)
                if lo & 0xE800 != 0xE800:
                    return
                off = ((hw & 0x7FF) << 12) | ((lo & 0x7FF) << 1)
                t = va + 4 + off - ((off & 0x400000) << 1)
                self.mark(va, 4, THUMB)
                if lo & 0xF800 == 0xE800:
                    t &= ~3
                    self.edges[va].add((t, "call"))
                    self.add_fn(t, "A", wl, "call")
                    if self.is_switch8(t):
                        ret = va + 4
                        n = self.s3[ret - BASE]
                        self.mark(ret, n + 2, LIT)
                        for i in range(n + 1):
                            tt = ret + 2 * self.s3[ret + 1 + i - BASE]
                            self.edges[va].add((tt | 1, "jmp"))
                            wl.append((tt, "T"))
                        return
                else:
                    self.edges[va].add((t | 1, "call"))
                    self.add_fn(t, "T", wl, "call")
                va += 4
                regs = {}
                continue
            if hw & 0xF800 in (0xE800, 0xF800):
                return
            self.mark(va, 2, THUMB)
            if hw & 0xF800 == 0xE000:
                off = (hw & 0x7FF) << 1
                t = va + 4 + off - ((off & 0x800) << 1)
                self.edges[va].add((t | 1, "jmp"))
                wl.append((t, "T"))
                return
            if hw & 0xF000 == 0xD000:
                c = (hw >> 8) & 0xF
                if c == 0xE:
                    return
                if c != 0xF:
                    off = (hw & 0xFF) << 1
                    t = va + 4 + off - ((off & 0x100) << 1)
                    self.edges[va].add((t | 1, "jmp"))
                    wl.append((t, "T"))
                va += 2
                continue
            if hw == 0x4778:  # bx pc
                wl.append(((va + 4) & ~3, "A"))
                return
            if hw & 0xFF00 == 0x4700:  # bx / blx reg
                self.ind.append((va, "T", hw))
                if hw & 0x80:
                    va += 2
                    continue
                return
            if hw & 0xFF00 == 0xBD00:  # pop {.., pc}
                return
            if hw & 0xFF87 in (0x4687, 0x4487):  # mov / add pc, reg
                self.ind.append((va, "T", hw))
                return
            if hw & 0xFF78 == 0x4478:  # add rd, pc
                rd = (hw & 7) | ((hw >> 4) & 8)
                if rd in regs:
                    t = (regs[rd] + va + 4) & 0xFFFFFFFF
                    if BASE <= t < BASE + S3_LEN:
                        self.edges[va].add((t, "lit"))
                        if self.code(t) and t & 1:
                            self.add_fn(t & ~1, "T", wl, "pcrel")
            if hw & 0xF800 == 0x4800:
                v = self.lit(va, ((va + 4) & ~3) + (hw & 0xFF) * 4)
                if v is not None:
                    regs[(hw >> 8) & 7] = v
            elif hw & 0xF800 == 0xA000:
                t = ((va + 4) & ~3) + (hw & 0xFF) * 4
                if BASE <= t < BASE + S3_LEN:
                    self.edges[va].add((t, "lit"))
            va += 2

    def walk_a(self, va: int, wl: deque) -> None:
        va &= ~3
        regs: dict[int, int] = {}
        while self.code(va):
            o = va - BASE
            if self.mode[o] in (ARM, THUMB):
                return
            w = self.u32(va)
            cond = w >> 28
            self.mark(va, 4, ARM)
            if cond == 0xF:
                if w & 0x0E000000 == 0x0A000000:  # blx imm
                    off = ((w & 0xFFFFFF) << 2) | ((w >> 23) & 2)
                    t = va + 8 + off - ((off & 0x2000000) << 1)
                    self.edges[va].add((t | 1, "call"))
                    self.add_fn(t, "T", wl, "call")
                    va += 4
                    continue
                if w & 0x0D70F000 == 0x0550F000:  # pld
                    va += 4
                    continue
                self.mode[o : o + 4] = bytes(4)
                return
            if w & 0x0E000000 == 0x0A000000:
                off = (w & 0xFFFFFF) << 2
                t = va + 8 + off - ((off & 0x2000000) << 1)
                if w & 0x01000000:
                    self.edges[va].add((t, "call"))
                    self.add_fn(t, "A", wl, "call")
                else:
                    self.edges[va].add((t, "jmp"))
                    wl.append((t, "A"))
                    if cond == 0xE:
                        return
                va += 4
                continue
            if w & 0x0FFFFFF0 == 0x012FFF10:  # bx reg
                self.ind.append((va, "A", w))
                if cond == 0xE:
                    return
                va += 4
                continue
            if w & 0x0FFFFFF0 == 0x012FFF30:  # blx reg
                self.ind.append((va, "A", w))
                va += 4
                continue
            if w & 0x0FFFFFF0 == 0x008FF100:  # add pc, pc, rm, lsl #2: branch table follows
                p = va + 4
                for _ in range(1024):
                    if not self.code(p) or (ww := self.u32(p)) & 0xFF000000 != 0xEA000000:
                        break
                    off = (ww & 0xFFFFFF) << 2
                    t = p + 8 + off - ((off & 0x2000000) << 1)
                    self.mark(p, 4, ARM)
                    self.edges[p].add((t, "jmp"))
                    wl.append((t, "A"))
                    p += 4
                return
            if w & 0x0FFFFFF0 == 0x079FF100:  # ldr pc, [pc, rm, lsl #2]: pointer table follows
                p = va + 8
                self.ind.append((va, "A", w))
                for _ in range(1024):
                    if not self.code(p) or self.mode[p - BASE] or not BASE <= (v := self.u32(p)) < CODE_END:
                        break
                    self.mark(p, 4, LIT)
                    self.edges[va].add((v, "jmp"))
                    wl.append((v & ~1, "T" if v & 1 else "A"))
                    p += 4
                va += 4
                continue
            rd, rn = (w >> 12) & 0xF, (w >> 16) & 0xF
            dp = w & 0x0C000000 == 0
            dp_test = dp and w & 0x01800000 == 0x01000000
            mul = w & 0x0E000090 == 0x00000090
            if w & 0x0F3F0000 == 0x051F0000:  # ldr rd, [pc, #imm]
                v = self.lit(va, va + 8 + ((w & 0xFFF) if w & 0x00800000 else -(w & 0xFFF)))
                if rd == 15:
                    if v is not None and self.code(v):
                        self.edges[va].add((v, "jmp"))
                        wl.append((v & ~1, "T" if v & 1 else "A"))
                    if cond == 0xE:
                        return
                elif v is not None:
                    regs[rd] = v
                va += 4
                continue
            if dp and not mul and not dp_test and not w & 0x02000000 and (w >> 21) & 0xF == 4 and w & 0xFF0 == 0:
                rm = w & 0xF  # add rd, rn, rm
                other = rm if rn == 15 else rn if rm == 15 else None
                if other is not None and other in regs:
                    t = (regs[other] + va + 8) & 0xFFFFFFFF
                    if BASE <= t < BASE + S3_LEN:
                        self.edges[va].add((t, "lit"))
                        if self.code(t):
                            self.add_fn(t & ~1, "T" if t & 1 else "A", wl, "pcrel")
            if w & 0x0FEF0000 == 0x028F0000:  # adr
                imm = _rot_imm(w)
                t = va + 8 + imm if (w >> 21) & 0xF == 4 else va + 8 - imm
                if BASE <= t < BASE + S3_LEN:
                    self.edges[va].add((t, "lit"))
            if rd == 15 and not dp_test and not mul and (dp or w & 0x0C000000 == 0x04000000):
                self.ind.append((va, "A", w))
                if cond == 0xE:
                    return
            elif w & 0x0E108000 == 0x08108000:  # ldm {.., pc}
                if cond == 0xE:
                    return
            elif w & 0x0E000010 == 0x06000010:  # undefined
                self.mode[o : o + 4] = bytes(4)
                return
            va += 4


def _words(s3: bytes) -> tuple[np.ndarray, np.ndarray]:
    """(src, value) of the big-endian word at every halfword offset."""
    h = np.frombuffer(s3, ">u2").astype(np.int64)
    return BASE + 2 * np.arange(len(h) - 1, dtype=np.int64), (h[:-1] << 16) | h[1:]


def _mask_out(src: np.ndarray, ranges: Iterable[Range]) -> np.ndarray:
    keep = np.ones(len(src), bool)
    for lo, hi in ranges:
        keep &= ~((src >= lo) & (src < hi))
    return keep


def build(s3: bytes, black: Iterable[int] = ()) -> CodeMap:
    """Descent from reset/main/vectors, then code pointers to a fixpoint, then orphan prologues."""
    cm = CodeMap(s3, black)
    roots = list(ROOTS)
    for a in range(*VECTORS, 4):
        v = struct.unpack_from(">I", s3, a - BASE)[0]
        if BASE <= v < CODE_END:
            roots.append((v & ~1, "T" if v & 1 else "A"))
    cm.run(roots, "root")

    src, val = _words(s3)
    keep = (val >= BASE) & (val < CODE_END) & _mask_out(src, (FPGA,))
    src, val = src[keep], val[keep]
    mode = np.frombuffer(cm.mode, np.uint8)
    done: set[int] = set()
    while True:
        m = mode[src - BASE]
        ok = ~((src < CODE_END) & ((m == ARM) | (m == THUMB)))
        vals, first = np.unique(val[ok], return_index=True)
        new = []
        for v in vals[np.argsort(first)].tolist():
            if v in done:
                continue
            done.add(v)
            if v & 1:
                t = v & ~1
                if cm.mode[t - BASE] != ARM and not _looks_arm(s3, t & ~3):
                    new.append((t, "T"))
            elif _looks_arm(s3, v):
                new.append((v, "A"))
        new = [(t, m) for t, m in new if not cm.mode[t - BASE]]
        if not new:
            break
        arm = [x for x in new if x[1] == "A"]
        cm.run(arm, "ptr")

        def thumb_ok(t: int) -> bool:
            if cm.mode[t - BASE]:
                return False
            p = (t & ~3) - 4
            if cm.mode[p - BASE] == ARM:  # only right after a terminating ARM insn
                w = struct.unpack_from(">I", s3, p - BASE)[0]
                return w >> 28 == 0xE and (
                    w & 0x0F000000 == 0x0A000000 or w & 0x0FFFFFF0 == 0x012FFF10 or w & 0x0E108000 == 0x08108000
                )
            return cm.mode[(t & ~3) - BASE] != ARM and cm.mode[(t & ~3) + 4 - BASE] != ARM

        th = [x for x in new if x[1] == "T" and thumb_ok(x[0])]
        cm.run(th, "ptr")
        if not arm and not th:
            break

    code = np.frombuffer(s3, np.uint8)[: CODE_END - BASE - 4]
    off = np.arange(len(code))
    push_t = (off % 2 == 0) & (code == 0xB5)
    push_a = (off % 4 == 0) & (code == 0xE9) & (np.roll(code, -1) == 0x2D) & (np.roll(code, -2) & 0x40 != 0)
    for o in np.nonzero(push_t | push_a)[0].tolist():
        if cm.mode[o]:
            continue
        va = BASE + o
        if code[o] == 0xB5:
            if not _looks_arm(s3, va & ~3):
                cm.run([(va, "T")], "orphan")
        else:
            cm.run([(va, "A")], "orphan")
    return cm


def collisions(cm: CodeMap) -> list[int]:
    """Thumb decoded right after a non-terminating ARM insn: one of the two descents is wrong."""
    out = []
    for a in range(0, CODE_END - BASE - 8, 4):
        if cm.mode[a] == ARM and cm.mode[a + 4] == THUMB:
            w = struct.unpack_from(">I", cm.s3, a)[0]
            if not (w >> 28 == 0xE and (w & 0x0F000000 == 0x0A000000 or w & 0x0FFFFFF0 == 0x012FFF10
                                         or w & 0x0E108000 == 0x08108000 or (w >> 12) & 0xF == 15)):
                out.append(BASE + a + 4)
    return out


def build_clean(s3: bytes, log=print) -> CodeMap:
    """Rebuild, blacklisting pointer/orphan roots whose descent collides with other code."""
    black: set[int] = set()
    for it in range(12):
        cm = build(s3, black)
        blame = {r for c in collisions(cm) if (r := cm.root.get(c)) is not None and cm.entry.get(r, ("", ""))[1] in ("ptr", "orphan")}
        log(f"  code map pass {it}: {len(cm.entry)} entries, blamed {len(blame - black)} roots")
        if not blame - black:
            return cm
        black |= blame
    return cm


def code_map(img: Image, log=print) -> CodeMap:
    """Cached `build_clean`."""
    key = hashlib.blake2b(img.s3, digest_size=10).hexdigest()
    path = cache_dir() / f"cmap1-{key}.pkl"
    if path.exists():
        cm = CodeMap(img.s3)
        cm.mode, cm.entry, edges, cm.ind, cm.black = pickle.loads(path.read_bytes())
        cm.edges.update(edges)
        return cm
    cm = build_clean(img.s3, log)
    path.write_bytes(pickle.dumps((cm.mode, cm.entry, dict(cm.edges), cm.ind, cm.black)))
    return cm


@dataclass
class Liveness:
    cm: CodeMap
    ents: list[int]
    live: set[int]
    roots: set[int]
    out: dict[int, set[int]] = field(repr=False)

    def fn_of(self, va: int) -> int | None:
        va &= ~1
        if not BASE <= va < CODE_END:
            return None
        i = bisect.bisect_right(self.ents, va) - 1
        return self.ents[i] if i >= 0 else None

    def fn_end(self, e: int) -> int:
        i = bisect.bisect_right(self.ents, e)
        return self.ents[i] if i < len(self.ents) else CODE_END

    def dead_ranges(self) -> list[tuple[int, int, list[int]]]:
        """Contiguous extents of dead functions."""
        out: list[tuple[int, int, list[int]]] = []
        for e in self.ents:
            if e in self.live:
                continue
            if out and out[-1][1] == e:
                out[-1] = (out[-1][0], self.fn_end(e), out[-1][2] + [e])
            else:
                out.append((e, self.fn_end(e), [e]))
        return out


def analyse(img: Image, dead_data: tuple[Range, ...] = (), log=print) -> Liveness:
    """Function graph over the code map; live = reachable from descent roots and code pointers outside `dead_data`."""
    cm = code_map(img, log)
    s3, mode = img.s3, cm.mode
    ents = sorted(cm.entry)
    lv = Liveness(cm, ents, set(), set(), {})
    fn_of = lv.fn_of
    out: defaultdict[int, set[int]] = defaultdict(set)

    def link(f: int | None, t: int) -> None:
        g = fn_of(t)
        if g is not None and g != f:
            out[f].add(g)

    for src, ts in cm.edges.items():
        f = fn_of(src)
        for t, k in ts:
            if k != "litslot":
                link(f, t)
    for a in range(0, CODE_END - BASE - 3, 4):
        if mode[a] == LIT:
            link(fn_of(BASE + a), struct.unpack_from(">I", s3, a)[0])
    # conservative: undecoded bytes inside an extent, swept linearly in the extent's mode
    for i, e in enumerate(ents):
        end = ents[i + 1] if i + 1 < len(ents) else CODE_END
        a = e
        while a < end - 3:
            o = a - BASE
            if mode[o]:
                a += 2
                continue
            if cm.entry[e][0] == "T":
                hw = (s3[o] << 8) | s3[o + 1]
                if hw & 0xF800 == 0xF000 and ((lo := (s3[o + 2] << 8) | s3[o + 3]) & 0xE800) == 0xE800:
                    off = ((hw & 0x7FF) << 12) | ((lo & 0x7FF) << 1)
                    link(e, a + 4 + off - ((off & 0x400000) << 1))
                elif hw & 0xF800 == 0x4800 and (la := ((a + 4) & ~3) + (hw & 0xFF) * 4) < BASE + S3_LEN - 4:
                    link(e, struct.unpack_from(">I", s3, la - BASE)[0])
                a += 2
            elif a % 4:
                a += 2
            else:
                w = struct.unpack_from(">I", s3, o)[0]
                if w & 0x0E000000 == 0x0A000000:
                    off = (w & 0xFFFFFF) << 2
                    link(e, a + 8 + off - ((off & 0x2000000) << 1))
                elif w & 0x0F3F0000 == 0x051F0000:
                    la = a + 8 + ((w & 0xFFF) if w & 0x00800000 else -(w & 0xFFF))
                    if BASE <= la < BASE + S3_LEN - 4:
                        link(e, struct.unpack_from(">I", s3, la - BASE)[0])
                a += 4

    # roots: descent roots + code pointers (to a decoded insn of matching mode) in data or undecoded code bytes
    lv.roots = {e for e, (_, o) in cm.entry.items() if o == "root"}
    m = np.frombuffer(mode, np.uint8)
    src, val = _words(s3)
    keep = (val >= BASE) & (val < CODE_END) & _mask_out(src, (FPGA, *dead_data))
    keep &= ~((src < CODE_END) & (m[np.clip(src - BASE, 0, S3_LEN - 1)] != 0))
    src, val = src[keep], val[keep]
    tm = m[(val & ~1) - BASE]
    ok = ((val & 1 == 1) & (tm == THUMB)) | ((val & 1 == 0) & (val % 4 == 0) & (tm == ARM))
    idx = np.searchsorted(np.asarray(ents, np.int64), val[ok] & ~1, side="right") - 1
    lv.roots |= {ents[i] for i in np.unique(idx[idx >= 0]).tolist()}

    dq = deque(lv.roots)
    while dq:
        f = dq.popleft()
        if f not in lv.live:
            lv.live.add(f)
            dq.extend(out[f] - lv.live)
    lv.out = dict(out)
    return lv


def code_runs(img: Image, lv: Liveness, used: Iterable[Range] = ()) -> list[Range]:
    """Dead byte runs of the code region: live extents plus the literals/strings live code reaches are kept."""
    keep = bytearray(CODE_END - BASE)
    for e in lv.live:
        keep[e - BASE : lv.fn_end(e) - BASE] = b"\1" * (lv.fn_end(e) - e)
    for src, ts in lv.cm.edges.items():
        if lv.fn_of(src) not in lv.live:
            continue
        for t, k in ts:
            t &= ~1
            if not BASE <= t < CODE_END or keep[t - BASE]:
                continue
            n = 2
            if k == "litslot":
                n = 4
            elif k == "lit":
                o = t - BASE
                j = img.s3.find(b"\0", o, o + 512)
                printable = j > o and all(32 <= c < 127 or c in (9, 10, 13) for c in img.s3[o:j])
                n = (j - o + 4) & ~3 if printable else 4
            keep[t - BASE : t - BASE + n] = b"\1" * n
    for lo, hi in used:
        keep[lo - BASE : hi - BASE] = b"\1" * (hi - lo)
    return [(BASE + m.start(), BASE + m.end()) for m in re.finditer(rb"\0+", bytes(keep))]


def data_runs(img: Image, lv: Liveness, dead_data: Iterable[Range] = ()) -> list[Range]:
    """Data-area spans whose only references come from dead code (span = up to the next referenced address)."""
    lo_d = FPGA[1]
    live_ref: dict[int, bool] = {}

    def ref(t: int, live: bool) -> None:
        if lo_d <= t < BSS:
            live_ref[t] = live_ref.get(t, False) or live

    for src, ts in lv.cm.edges.items():
        f_live = lv.fn_of(src) in lv.live
        for t, k in ts:
            if k == "lit":
                ref(t, f_live)
    for a in range(0, CODE_END - BASE - 3, 4):
        if lv.cm.mode[a] == LIT:
            ref(struct.unpack_from(">I", img.s3, a)[0], lv.fn_of(BASE + a) in lv.live)
    w = np.frombuffer(img.s3, ">u4").astype(np.int64)
    va = BASE + 4 * np.arange(len(w), dtype=np.int64)
    sel = (va >= lo_d) & (va < BSS) & (w >= lo_d) & (w < BSS) & _mask_out(va, dead_data)
    for t in np.unique(w[sel]).tolist():
        ref(t, True)
    keys = sorted(live_ref)
    runs: list[Range] = []
    for a, b in zip(keys, keys[1:] + [BSS]):
        if live_ref[a]:
            continue
        if runs and runs[-1][1] == a:
            runs[-1] = (runs[-1][0], b)
        else:
            runs.append((a, b))
    return runs


@dataclass(frozen=True)
class Candidate:
    start: int
    end: int
    kind: str
    fns: int
    hint: str

    def tsv(self) -> str:
        return f"{self.start:#x}\t{self.end:#x}\t{self.end - self.start}\t{self.kind}\t{self.fns}\t{self.hint}"


TSV_HEADER = "start\tend\tsize\tkind\tfns\thint"


def candidates(img: Image, lv: Liveness, stock: Liveness | None = None, min_size: int = 512,
               dead_data: tuple[Range, ...] = (), used: tuple[Range, ...] = ()) -> list[Candidate]:
    """Dead code runs (newly dead vs the stock image, or stock orphans) and dead-only data spans, largest first."""
    out = []
    for a, b in code_runs(img, lv, used):
        if b - a < min_size:
            continue
        fs = lv.ents[bisect.bisect_left(lv.ents, a) : bisect.bisect_left(lv.ents, b)]
        newly = stock is not None and any(f in stock.live for f in fs)
        hint = re.search(rb"[A-Za-z_][\x20-\x7e]{9,}", img.s3[a - BASE : b - BASE])
        out.append(Candidate(a, b, "code:newly-dead" if newly else "code:orphan", len(fs), hint.group().decode()[:60] if hint else ""))
    for a, b in data_runs(img, lv, dead_data):
        if b - a >= min_size:
            out.append(Candidate(a, b, "data:dead-refs-only", 0, ""))
    return sorted(out, key=lambda c: c.start - c.end)
