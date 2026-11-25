"""python -m tools.re <command>: DP2M M6M reverse-engineering CLI."""

from __future__ import annotations

import argparse
import bisect
import hashlib
import re
import sys
from functools import cached_property
from itertools import islice
from pathlib import Path

import numpy as np

from . import liveness, nvram, stock, ui
from .disasm import Annotator
from .emu import EmuError, Machine
from .image import BASE, END, FLAGS, FPGA, Image, Package, load_syms
from .xref import CALLS, KINDS, Index, fn_start

ROOT = Path(__file__).resolve().parents[2]
FW = ROOT / "data" / "DP2MV105.bin"


def num(s: str) -> int:
    return int(s, 0)


def span(s: str) -> tuple[int, int]:
    lo, hi = s.split(":")
    return num(lo), num(hi)


def log(s: str) -> None:
    print(s, file=sys.stderr, flush=True)


class Ctx:
    """Lazily loaded image, symbols, xref index and emulator for one invocation."""

    def __init__(self, a: argparse.Namespace):
        self.a = a

    @cached_property
    def stock(self) -> Image:
        return Image.load(self.a.fw)

    @cached_property
    def img(self) -> Image:
        return Image.load(self.a.image) if self.a.image else self.stock

    @cached_property
    def syms(self) -> dict:
        p = self.a.syms or (Path(self.a.image).parent / "m6m" / "m6m.syms" if self.a.image else None)
        return load_syms(p) if p and Path(p).exists() else {}

    @cached_property
    def names(self) -> dict[int, str]:
        return {**stock.NAMES, **{s.va: n for n, s in self.syms.items()}}

    @cached_property
    def sized(self) -> list[tuple[int, int, str]]:
        return sorted((s.va, s.va + s.size, n) for n, s in self.syms.items() if s.size)

    @cached_property
    def index(self) -> Index:
        return Index.of(self.img)

    @cached_property
    def machine(self) -> Machine:
        m = Machine(self.img)
        m.load_nv_defaults()
        return m

    def where(self, va: int) -> str:
        """Containing payload symbol, else nearest Thumb prologue (named when known)."""
        i = bisect.bisect_right(self.sized, (va, 1 << 32)) - 1
        if i >= 0 and self.sized[i][0] <= va < self.sized[i][1]:
            lo, _, n = self.sized[i]
            return f"{n}+{va - lo:#x}"
        f = fn_start(self.img, va) if BASE <= va < END else None
        return f"fn {f:#x} {self.names.get(f, '')}".rstrip() if f is not None else ""


def cmd_package(c: Ctx) -> None:
    path = Path(c.a.path or c.a.fw)
    pkg = Package(path.read_bytes())
    ref = Package(Path(c.a.fw).read_bytes()) if path.resolve() != Path(c.a.fw).resolve() else None
    print(f"{path.name}: {len(pkg.raw)} B  version {pkg.version} {pkg.versions}  date {pkg.date}  m6m @ {pkg.c78_off:#x}")
    print(f"flags {pkg.flags:#04x}: " + "; ".join(f"b{b} {d}" for b, d in FLAGS.items() if pkg.flags >> b & 1))
    for name, (off, n) in pkg.segments().items():
        seg = pkg.segment(name)
        same = "" if ref is None else "  stock" if seg == ref.segment(name) else "  modified"
        print(f"  {name:<7} {off:#010x} {n:#09x}  {hashlib.blake2b(seg, digest_size=8).hexdigest()}{same}")


def cmd_dis(c: Ctx) -> None:
    ann = Annotator(c.img, c.names)
    end = c.a.end if c.a.end is not None else min(END, c.a.va + 4 * c.a.n)
    lines = ann.lines(c.a.va, end, c.a.arm)
    for line in lines if c.a.end is not None else islice(lines, c.a.n):
        print(line)


def _refs(c: Ctx, value: int, kinds: tuple[str, ...] | None) -> None:
    for r in c.index.refs(value, kinds):
        slot = f" slot {r.slot:#x}" if r.slot != r.src else ""
        print(f"{r.src:08x} {r.kind:<5}{slot}  {c.where(r.src)}")


def cmd_xrefs(c: Ctx) -> None:
    _refs(c, c.a.value, tuple(c.a.kinds.split(",")) if c.a.kinds else None)


def cmd_callers(c: Ctx) -> None:
    _refs(c, c.a.va, CALLS)


def _text(m: Machine, sid: int, lang: int = 0) -> str | None:
    try:
        return ui.string(m, sid, lang)
    except EmuError as e:
        return f"<{e}>"


def cmd_sid(c: Ctx) -> None:
    m = c.machine
    sids = c.a.sid
    if c.a.find:
        extra = m.u32(c.syms["dp2m_string_count"].va) if "dp2m_string_count" in c.syms else 0
        sids = [s for s in range(stock.STOCK_SIDS + extra) if c.a.find.lower() in (_text(m, s, c.a.lang) or "").lower()]
    for s in sids:
        print(f"{s:4d} {s:#05x}  {_text(m, s, c.a.lang)!r}")


def cmd_strings(c: Ctx) -> None:
    pat = re.compile(c.a.pattern, re.I) if c.a.pattern else None
    s3 = c.img.s3
    for m in re.finditer(rb"[\x20-\x7e\t\n]{%d,}(?=\0)" % c.a.min, s3):
        va, text = BASE + m.start(), m.group().decode("latin1")
        if FPGA[0] <= va < FPGA[1] or (pat and not pat.search(text)):
            continue
        refs = f" {len(c.index.refs(va, ('tlit', 'alit', 'tadr', 'aadr', 'w'))):3d}" if c.a.refs else ""
        print(f"{va:08x}{refs}  {text!r}")


def cmd_menus(c: Ctx) -> None:
    m = c.machine
    tables = ui.menu_tables(m, c.a.ids or range(0x200))
    by: dict[int, list[int]] = {}
    for mid, t in tables.items():
        by.setdefault(t, []).append(mid)
    for t, mids in sorted(by.items(), key=lambda x: x[1][0]):
        rows = ui.rows(m, t)
        print(f"menu {' '.join(f'{i:#x}' for i in mids)} @ {t:#x} {c.names.get(t, '')} ({len(rows)} rows)")
        for k, r in enumerate(rows):
            text = _text(m, r.sid) if r.sid else None
            print(f"  {k:2d} id {r.id:#06x} sid {r.sid:4d} {text!r:30} v={r.value:<3d} icons qs {r.qs_icon:#x} strip {r.strip_icon:#x} "
                  f"osd {r.osd_icon:#x}  get {c.names.get(r.get & ~1, hex(r.get))} set {c.names.get(r.set & ~1, hex(r.set))} extra {r.extra:#x}")


def cmd_screens(c: Ctx) -> None:
    for i, h in ui.screens(c.img):
        print(f"{i:3d} {stock.SCREEN_VT + 4 * i:#x} -> {h:#x} {c.names.get(h & ~1, '')}")


def cmd_icons(c: Ctx) -> None:
    table = ui.icons(c.img)
    ids = c.a.ids or sorted(table)
    tiles = []
    for i in ids:
        ic = table.get(i)
        if ic is None:
            print(f"{i:#05x} -")
            continue
        px = ic.pixels(c.img)
        print(f"{i:#05x} {ic.bits:#x} {ic.w}x{ic.h} levels {np.unique(px).tolist()[:18]}")
        tiles.append((f"{i:#x} {ic.w}x{ic.h}", px))
    if c.a.sheet and tiles:
        ui.contact_sheet(tiles, c.a.sheet, ui.palette(c.img), cols=c.a.cols, scale=c.a.scale)
        print(f"wrote {c.a.sheet}")


def cmd_text(c: Ctx) -> None:
    cov = c.machine.text(c.a.text, c.a.px, c.a.style, lang=c.a.lang)
    ink = np.argwhere(cov)
    if not len(ink):
        print("no ink")
        return
    (y0, x0), (y1, x1) = ink.min(0), ink.max(0)
    print(f"ink x {x0}..{x1} y {y0}..{y1} ({x1 - x0 + 1}x{y1 - y0 + 1}) levels {np.unique(cov).tolist()}")
    if c.a.out:
        crop = cov[max(0, y0 - 2) : y1 + 3, max(0, x0 - 2) : x1 + 3]
        ui.contact_sheet([(repr(c.a.text), crop)], c.a.out, ui.palette(c.img), cols=1, scale=c.a.scale)
        print(f"wrote {c.a.out}")


def cmd_liveness(c: Ctx) -> None:
    dead = tuple(c.a.dead)
    lv = liveness.analyse(c.img, dead, log)
    base = None if c.img is c.stock else liveness.analyse(c.stock, (), log)
    dr = lv.dead_ranges()
    log(f"functions {len(lv.ents)}, live {len(lv.live)}, dead extents {len(dr)} / {sum(b - a for a, b, _ in dr)} B")
    rows = liveness.candidates(c.img, lv, base, c.a.min, dead, tuple(c.a.used))
    out = open(c.a.out, "w") if c.a.out else sys.stdout
    with out:
        print(liveness.TSV_HEADER, file=out)
        for r in rows:
            print(r.tsv(), file=out)
    log(f"{len(rows)} candidates, {sum(r.end - r.start for r in rows)} B" + (f" -> {c.a.out}" if c.a.out else ""))


def cmd_nvram(c: Ctx) -> None:
    lo, hi = c.a.window
    acc, calls = nvram.scan(c.img, lo, hi, c.index)
    log(f"{len(acc)} accesses, {len(calls)} calls")
    for line in nvram.report(c.img, acc, calls, c.a.block, c.a.len, stock.NV_DEFAULTS, tuple(c.a.offset)):
        print(line)


def main(argv: list[str] | None = None) -> None:
    ap = argparse.ArgumentParser(prog="python -m tools.re", description=__doc__)
    ap.add_argument("--fw", type=Path, default=FW, help="stock DP2MV105.bin (default: %(default)s)")
    ap.add_argument("--image", type=Path, help="image to analyse: built C78FRM.bin or package (default: --fw)")
    ap.add_argument("--syms", type=Path, help="m6m.syms (default: <image dir>/m6m/m6m.syms)")
    sub = ap.add_subparsers(dest="cmd", required=True)

    p = sub.add_parser("package", help="package header, flags & segments (vs --fw)")
    p.add_argument("path", nargs="?", type=Path)
    p = sub.add_parser("dis", help="disassemble with literal/call annotation")
    p.add_argument("va", type=num)
    p.add_argument("end", type=num, nargs="?")
    p.add_argument("-n", type=int, default=40, help="instructions when END is omitted")
    p.add_argument("--arm", action="store_true")
    p = sub.add_parser("xrefs", help="references to a value: calls, literal loads, adr, data pointers")
    p.add_argument("value", type=num)
    p.add_argument("--kinds", help=f"comma list of {','.join(KINDS)}")
    p = sub.add_parser("callers", help="BL/BLX callers")
    p.add_argument("va", type=num)
    p = sub.add_parser("sid", help="UI strings by id (through the firmware lookup)")
    p.add_argument("sid", type=num, nargs="*")
    p.add_argument("--find", help="case-insensitive substring")
    p.add_argument("--lang", type=int, default=0)
    p = sub.add_parser("strings", help="C strings in S3 (outside the FPGA bitstream)")
    p.add_argument("pattern", nargs="?", help="regex filter")
    p.add_argument("--min", type=int, default=4)
    p.add_argument("--refs", action="store_true", help="count literal/adr/data references")
    p = sub.add_parser("menus", help="Get_Menu_Table emulation & row dump")
    p.add_argument("ids", type=num, nargs="*")
    sub.add_parser("screens", help="screen vtable (posted id -> handler)")
    p = sub.add_parser("icons", help="icon table decode & PNG contact sheet")
    p.add_argument("ids", type=num, nargs="*")
    p.add_argument("--sheet", type=Path)
    p.add_argument("--cols", type=int, default=10)
    p.add_argument("--scale", type=int, default=2)
    p = sub.add_parser("text", help="render text offscreen with the stock font engine")
    p.add_argument("text")
    p.add_argument("--px", type=int, default=40)
    p.add_argument("--style", type=num, default=0x22, help="0x21 left, 0x22 centre, 0x24 right")
    p.add_argument("--lang", type=int, default=0)
    p.add_argument("--out", type=Path)
    p.add_argument("--scale", type=int, default=3)
    p = sub.add_parser("liveness", help="code map + reachability -> dead-region candidates TSV")
    p.add_argument("--out", type=Path)
    p.add_argument("--min", type=int, default=512, help="minimum candidate size")
    p.add_argument("--dead", type=span, action="append", default=[], metavar="LO:HI", help="data known dead (not a root)")
    p.add_argument("--used", type=span, action="append", default=[], metavar="LO:HI", help="code already reclaimed")
    p = sub.add_parser("nvram", help="static NVRAM block coverage")
    p.add_argument("--window", type=span, default=(0xA0939600, 0xA0939B00), metavar="LO:HI")
    p.add_argument("--block", type=num, default=stock.NV_BLOCK)
    p.add_argument("--len", type=num, default=stock.NV_LEN)
    p.add_argument("--offset", type=num, action="append", default=[], help="list users of a block offset")

    a = ap.parse_args(argv)
    globals()[f"cmd_{a.cmd}"](Ctx(a))


if __name__ == "__main__":
    main()
