"""Verifier plumbing: build output loader, PASS/FAIL report, NOR stubs shared by the suites."""

from __future__ import annotations

import re
from dataclasses import dataclass
from pathlib import Path

from tools.re import stock
from tools.re.emu import Machine
from tools.re.image import Image, Package, Sym, load_syms


class Skip(Exception):
    """Suite does not apply to this build."""


def _ints(s: str) -> list[int]:
    return [int(x, 0) for x in re.findall(r"-?(?:0x[0-9A-Fa-f]+|\d+)", s)]


@dataclass(frozen=True)
class Generated:
    """Tables of the builder's `m6m/catalog.c`: what the image must contain."""

    strings: list[str]
    films: list[tuple[int, ...]]  # packed, paper class, auto paper, mono
    looks: list[tuple[tuple[int, ...], bytes, int]]  # cmcm, knots, flags per packed mode from 9
    tv_min: tuple[int, ...]
    menus: dict[str, list[list[str]]]  # m_<name> -> row initializer fields

    @classmethod
    def parse(cls, text: str) -> Generated:
        def block(name: str) -> list[str]:
            body = re.search(rf"\b{name}\[\d*\][^=]*= \{{\n(.*?)\n\}};", text, re.S)[1]
            return [line.split("/*")[0] for line in body.splitlines()]

        strings = [re.sub(r'\\(["\\])', r"\1", m[1]) for line in block("dp2m_strings") if (m := re.match(r'\s*"(.*)",$', line))]
        looks = [(tuple(v[:9]), bytes(v[9:27]), v[27]) for v in map(_ints, block("dp2m_lv_looks"))]
        menus = {}
        for name, body in re.findall(r"struct menu_row (m_\w+)\[\][^=]*= \{\n(.*?)\n\};", text, re.S):
            rows = [[f.strip() for f in line.strip().strip("{},").split(",")] for line in body.splitlines()]
            menus[name] = [row for row in rows if row != ["0"]]
        tv = re.search(r"dp2m_tv_min_q16\[7\] = \{(.*?)\}", text)[1]
        return cls(strings, [tuple(_ints(line)) for line in block("dp2m_films")], looks, tuple(_ints(tv)), menus)


@dataclass
class Build:
    dir: Path
    stock: Image
    img: Image
    pkg: Package
    pkg_name: str
    syms: dict[str, Sym]
    cat: dict[str, int]  # catalog.h defines
    gen: Generated  # catalog.c tables

    @classmethod
    def load(cls, out: Path, fw: Path) -> Build:
        pkgs = sorted(out.glob("DP2MV*.bin"))
        if len(pkgs) != 1:
            raise SystemExit(f"{out}: expected one DP2MVxxx.bin, found {[p.name for p in pkgs]}")
        cat = {m[1]: int(m[2], 0) for m in re.finditer(r"^#define (\w+) (\w+?)u?$", (out / "m6m/catalog.h").read_text(), re.M)}
        return cls(out, Image.load(fw), Image.load(out / "C78FRM.bin"), Package(pkgs[0].read_bytes()), pkgs[0].name,
                   load_syms(out / "m6m/m6m.syms"), cat, Generated.parse((out / "m6m/catalog.c").read_text()))

    def sym(self, name: str) -> int:
        """Symbol address, Thumb bit cleared."""
        return self.syms[name].va

    def require(self, *names: str) -> None:
        if missing := [n for n in names if n not in self.syms]:
            raise Skip(f"symbols absent: {', '.join(missing)}")

    def value(self, field: str) -> int:
        """A catalog.c initializer field: number, G(addr) / S(addr) cast or symbol."""
        if m := re.fullmatch(r"[GS]\((\w+)\)", field):
            field = m[1]
        return self.syms[field].addr if field in self.syms else int(field, 0)


class Report:
    """PASS/FAIL lines of one suite; `artifacts` is where suites may write images."""

    def __init__(self, artifacts: Path | None = None) -> None:
        self.artifacts = artifacts
        self.passed = self.failed = 0
        self.fails: list[str] = []

    def check(self, ok: bool, what: str) -> bool:
        print(f"  {'PASS' if ok else 'FAIL'} {what}", flush=True)
        if ok:
            self.passed += 1
        else:
            self.failed += 1
            self.fails.append(what)
        return ok

    def eq(self, got: object, exp: object, what: str) -> bool:
        return self.check(got == exp, what if got == exp else f"{what}: got {got!r}, expected {exp!r}")


class Nor:
    """NOR bank 0 and heap stubs for the stock NVRAM load (reads the 2 KB bank) and flush (writes from offset 8)."""

    BUF = 0xA1E00000  # malloc result, above the payload RAM
    HELPERS = ((0xA001B020, None), (0xA001B026, 0), (0xA0143336, None), (0xA0145650, 0))  # record counter set/get, sync, busy

    def __init__(self, m: Machine):
        self.m, self.data, self.writes = m, bytearray(0x800), []
        m.stub(stock.MALLOC, ret=self.BUF)
        m.stub(stock.FREE)
        m.stub(stock.NOR_READ, on=self._read)
        m.stub(stock.NOR_WRITE, on=self._write)
        for va, ret in self.HELPERS:
            m.stub(va, ret=ret)

    def _read(self, a: list[int]) -> None:
        off, n, buf = a[:3]
        self.m.wr(buf, bytes(self.data[off : off + n]))

    def _write(self, a: list[int]) -> None:
        off, n, buf = a[:3]
        self.writes.append((off, n, self.m.rd(buf, n)))
