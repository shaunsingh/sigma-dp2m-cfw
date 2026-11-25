"""m6m suite: payload behaviour in the built C78FRM.bin and the DP2MVxxx.bin package, driven by m6m.syms + catalog.h."""

from __future__ import annotations

import re
import struct
from dataclasses import dataclass
from itertools import product

from tools.re import stock, ui
from tools.re.emu import SCRATCH, STACK, EmuError, Machine, outside
from tools.re.image import BSS, END, FPGA, IPL_LEN

from .harness import Build, Nor, Report

FILM_MODE0 = 9
NV_RESERVED = (stock.NV_BLOCK + 0x04, stock.NV_BLOCK + 0x14)  # zero-default bytes free for payload settings
LV_DIRTY = (0xA07B1398, 0xA07B139C)  # LV refresh flags, set by the paper setters
LV_CMCM, LV_MONO_Q7, LV_KNOTS = 1, 2, 4
PAPER_NV = {1: "color_paper", 2: "bw_paper", 3: "lut_slot"}
PAGE_RESET, PAGE_LIST = 0xA0058A0A, 0xA07F230E  # menu entry copies the page template (state + 0x14) to the live list


@dataclass(frozen=True)
class Film:
    cm: int
    packed: int
    paper_cls: int
    auto_paper: int
    mono: int


def s32(v: int) -> int:
    return v - (1 << 32) if v & 0x80000000 else v


def films(b: Build) -> list[Film]:
    return [Film(FILM_MODE0 + i, *f) for i, f in enumerate(b.gen.films)]


def menu_map(b: Build) -> dict[int, int]:
    """Generated menus registered by id (`m_<hex id>`): id -> rows."""
    return {int(n[2:], 16): b.sym(n) for n in b.gen.menus if re.fullmatch(r"m_[0-9a-f]+", n)}


def packed_of(f: Film | None, cm: int, nv: dict[str, int]) -> int:
    if f is None:
        return cm
    if not f.paper_cls:
        return f.packed
    p = nv.get(PAPER_NV[f.paper_cls], 0)
    return f.packed + (p - 1 if 1 <= p <= 8 else f.auto_paper)


def expand_knots(k: bytes) -> bytes:
    """18 knots at x = 0..128 step 16, 192..384 step 64, 512..1023 step 128 -> 1024 B curve."""
    out, i = [], 0
    for sh, cnt in ((4, 8), (6, 4), (7, 5)):
        for _ in range(cnt):
            y0, dy = k[i], k[i + 1] - k[i]
            out += [(y0 + ((dy * s + (1 << sh >> 1)) >> sh)) & 0xFF for s in range(1 << sh)]
            i += 1
    return bytes(out)


def lv_matrix(cmcm: tuple[int, ...], flags: int, cc: list[int]) -> tuple[int, ...]:
    """B2Y matrix the CMCM hook programs: film q10 x live q7 CCMatrix (or direct q7 mono weights), BT.601 Y row."""
    m = [0] * 9 + [0x4D, 0x96, 0x1D, 0x200, 0]
    for i, j in product(range(3), range(3)):
        if flags & LV_MONO_Q7:
            v = cmcm[i * 3 + j]
        else:
            v = (cmcm[i * 3] * cc[j] + cmcm[i * 3 + 1] * cc[3 + j] + cmcm[i * 3 + 2] * cc[6 + j] + 512) >> 10
            v = max(-2047, min(2047, v))
        m[i * 3 + j] = v & 0xFFF
    return tuple(m)


def check_invariants(b: Build, r: Report) -> None:
    r.check(b.img.c78[:IPL_LEN] == b.stock.c78[:IPL_LEN], "IPL is stock")
    r.check(b.img.rd(FPGA[0], FPGA[1] - FPGA[0]) == b.stock.rd(FPGA[0], FPGA[1] - FPGA[0]), "FPGA bitstream is stock")
    r.check(b.img.rd(BSS, END - BSS) == b.stock.rd(BSS, END - BSS), ".bss tail is stock")


def check_menus(b: Build, r: Report) -> None:
    m, ms = Machine(b.img), Machine(b.stock)
    m.load_nv_defaults()
    ms.load_nv_defaults()
    gen = menu_map(b)
    film_page, paper, paper0 = b.cat["MENU_FILM_PAGE"], b.cat["MENU_PAPER"], b.cat["MENU_PAPER_CLS0"]
    r.check(film_page in gen, f"Film page {film_page:#x} in the generated menu map")
    names = {s.va: n for n, s in b.syms.items()}
    for mid, rows in sorted(gen.items()):
        r.eq(ui.menu_table(m, mid), rows, f"Get_Menu_Table({mid:#x}) -> {names.get(rows, hex(rows))}")
    for name, rows in b.gen.menus.items():
        got = [row.w for row in ui.rows(b.img, b.sym(name))]
        exp = [tuple(b.value(f) for f in row) for row in rows]
        bad = next((k for k in range(max(len(got), len(exp))) if got[k : k + 1] != exp[k : k + 1]), None)
        r.check(bad is None, f"{name}: {len(exp)} rows = catalog.c" + (f" (row {bad} differs)" if bad is not None else ""))
    subs = {row.id: ui.menu_table(m, row.id) for row in ui.rows(m, gen[film_page])}
    r.check(all(subs.values()), f"Film page submenus resolve {[hex(k) for k in subs]}")
    m.wr(PAGE_LIST, bytes(22))
    m.call(PAGE_RESET)
    r.eq(list(struct.unpack(">11H", m.rd(PAGE_LIST, 22))), [0x0E, 0x0F, film_page, *range(0x10, 0x18)], "menu page list after the entry reset 0xA0058A0A")

    fl = films(b)
    for cls in range(4):
        cm = next((f.cm for f in fl if f.paper_cls == cls), 0)
        m.w8(stock.NV_CM, cm)
        r.eq(ui.menu_table(m, paper), gen[paper0 + cls], f"Get_Menu_Table({paper:#x}) for cm {cm} -> paper class {cls}")
    m.load_nv_defaults()

    iso = {name: b.sym(name) for name in ("m_iso_1ev", "m_iso_3ev") if name in b.syms}
    for step, name in enumerate(("m_iso_1ev", "m_iso_3ev")):
        if name in iso:
            m.w8(stock.NV_ISO_STEP, step)
            r.eq(ui.menu_table(m, stock.MENU_ISO), iso[name], f"Get_Menu_Table({stock.MENU_ISO:#x}) with ISO step {step} -> {name}")
    m.load_nv_defaults()
    st, bt = ui.menu_tables(ms), ui.menu_tables(m)
    changed = [i for i in sorted(set(st) | set(bt)) if i not in gen and i != paper and bt.get(i) not in iso.values() and st.get(i) != bt.get(i)]
    r.eq(changed, [], f"other menu ids resolve as stock ({len(st)} stock ids)")


def check_strings(b: Build, r: Report) -> None:
    m, ms = Machine(b.img), Machine(b.stock)
    m.load_nv_defaults()
    ms.load_nv_defaults()
    strs = b.gen.strings
    r.eq(b.img.u32(b.sym("dp2m_string_count")), len(strs), "dp2m_string_count = catalog.c")
    sid0, title = stock.STOCK_SIDS, b.cat["SID_PAPER_TITLE"]
    bad = []
    for i, s in enumerate(strs):
        exp = strs[i + 1] if sid0 + i == title else s  # NV defaults: stock color mode -> paper class 0
        bad += [(sid0 + i, lang) for lang in (0, 1, 16) if ui.string(m, sid0 + i, lang) != exp]
    r.eq(bad, [], f"UI string lookup of all {len(strs)} dp2m strings (languages 0, 1, 16)")
    fl = films(b)
    for cls in range(4):
        cm = next((f.cm for f in fl if f.paper_cls == cls), 0)
        m.w8(stock.NV_CM, cm)
        r.eq(ui.string(m, title), strs[title - sid0 + 1 + cls], f"paper title for cm {cm} (class {cls})")
    m.load_nv_defaults()
    diff = []
    for sid, lang in product(range(sid0), (0, 1)):
        try:
            exp = ui.string(ms, sid, lang)
        except EmuError:
            continue
        if ui.string(m, sid, lang) != exp:
            diff.append((sid, lang))
    r.eq(diff, [], f"stock sids 0..{sid0 - 1} unchanged (languages 0, 1)")


def check_settings(b: Build, r: Report) -> dict[str, int]:
    """Setter/getter pairs: own NVRAM byte, value round trip, clamping, persistence through the stock flush + load."""
    m = Machine(b.img)
    m.load_nv_defaults()
    values: dict[int, set[int]] = {}
    for rows in menu_map(b).values():
        for row in ui.rows(b.img, rows):
            values.setdefault(row.get & ~1, set()).add(row.value)
    names = sorted(n[7:] for n in b.syms if n.startswith("nv_get_") and n != "nv_get_none")
    addrs, vals = {}, {}
    for name in names:
        get, put = b.sym(f"nv_get_{name}"), b.sym(f"nv_set_{name}")
        vs = sorted(values.get(get, ()))
        if not r.check(len(vs) > 1, f"nv {name}: menu values {vs}"):
            continue
        before = m.rd(stock.NV_BLOCK, stock.NV_LEN)
        with m.writes(STACK, LV_DIRTY) as log:
            m.call(put, vs[-1])
        after = m.rd(stock.NV_BLOCK, stock.NV_LEN)
        changed = [stock.NV_BLOCK + i for i in range(stock.NV_LEN) if before[i] != after[i]]
        addr = changed[0] if len(changed) == 1 else None
        ok = addr is not None and NV_RESERVED[0] <= addr < NV_RESERVED[1] and outside(log, (addr, addr + 1)) == []
        r.check(ok, f"nv_set_{name} writes only NVRAM {addr and hex(addr)} (reserved +0x04..+0x13)")
        if addr is None:
            continue
        rt = []
        for v in vs:
            m.call(put, v)
            if m.call(get) != v:
                rt.append(v)
        m.w8(addr, vs[-1] + 1)
        r.check(not rt and m.call(get) == 0, f"nv {name}: {vs[0]}..{vs[-1]} round-trip, {vs[-1] + 1} reads as 0")
        addrs[name], vals[name] = addr, vs[-1]
    r.eq(len(set(addrs.values())), len(addrs), "settings use distinct NVRAM bytes")

    nor = Nor(m)
    for name, v in vals.items():
        m.call(b.sym(f"nv_set_{name}"), v)
    m.call(stock.NV_FLUSH)
    off, n, buf = nor.writes[-1] if nor.writes else (0, 0, b"")
    rec = {name: buf[0x18 + a - stock.NV_BLOCK] for name, a in addrs.items()} if off == 8 else {}
    r.eq(rec, vals, "stock flush writes the settings into the NOR record (block at NOR 0x20)")
    nor.data[0:4] = struct.pack(">I", 0x20120918)  # record magic, kept outside the flushed range
    nor.data[off : off + n] = buf
    m.wr(stock.NV_BLOCK, bytes(stock.NV_PERSIST))
    m.call(stock.NV_LOAD, stock.NV_BLOCK)
    r.eq({name: m.call(b.sym(f"nv_get_{name}")) for name in vals}, vals, "settings survive NOR flush + boot load")
    return addrs


def check_bracketing(b: Build, r: Report) -> None:
    m = Machine(b.img)
    for idx in (1, 3):
        m.w16(stock.BKT_STEP_IDX, idx)
        step = s32(b.img.u32(stock.BKT_STEP_TBL + 4 * idx))
        for order, ev in ((0, [0, 1, -1]), (4, [0, 2, 1, -1, -2]), (6, [2, 1, 0, -1, -2]), (7, [-2, -1, 0, 1, 2])):
            m.w8(stock.NV_BKT_ORD, order)
            got = (m.call(stock.BKT_COUNT), [s32(m.call(stock.BKT_EV_Q16, i)) for i in range(len(ev))])
            r.eq(got, (len(ev), [e * step for e in ev]), f"bracketing order {order} step {step / 65536:.3f} EV")


def check_min_shutter(b: Build, r: Report, nv: int) -> None:
    m = Machine(b.img)
    tv = b.gen.tv_min
    a_bad, p_bad = [], []
    for idx in (*range(7), 7, 0xFF):
        m.w8(nv, idx)
        m.run(stock.AUTOISO_A, stock.AUTOISO_A + 14, r0=0, r1=0x12345678)
        if (m.reg("r0"), m.reg("r1")) != (tv[idx if idx <= 6 else 0], 0x12345678):
            a_bad.append(idx)
        for site in stock.AUTOISO_P:
            m.run(site, site + 4, r0=0)
            if m.reg("r0") != (tv[idx] + 0x30000 if 1 <= idx <= 6 else 0x90000):
                p_bad.append((site, idx))
    r.eq(a_bad, [], "ISO Auto min shutter, A mode: table value, idx > 6 -> Auto, r1 kept")
    r.eq(p_bad, [], "ISO Auto min shutter, P mode (both sites): table value + 3 EV, Auto -> stock 9.0")


def check_ipc(b: Build, r: Report, addrs: dict[str, int]) -> None:
    m = Machine(b.img)
    m.load_nv_defaults()
    cmd, meta = SCRATCH + 0x100, SCRATCH + 0x200
    fl = {f.cm: f for f in films(b)}
    regs_bad: list[tuple] = []

    def pack(cm: int, cs: int, **nv: int) -> tuple[int, int]:
        for k, v in nv.items():
            m.w8(addrs[k], v)
        m.wr(meta + 0x1D, bytes([cs, cm]))
        m.w32(STACK[1] + 0x14, meta)
        m.wr(cmd, bytes(8))
        m.run(stock.IPC_CMD20, stock.IPC_CMD20 + 6, r0=0x55, r1=0x3F800000, r4=cmd)
        if (m.reg("r0"), m.reg("r1")) != (0, 0x3F800000):
            regs_bad.append((cm, cs, nv))
        return m.u8(cmd + 5), m.u8(cmd + 6)

    zero = {k: 0 for k in addrs if k in ("engine", "grain", "halation", "fill", *PAPER_NV.values())}
    bad = [cm for cm in range(FILM_MODE0) if pack(cm, 0, **zero)[0] != cm]
    r.eq(bad, [], "cmd[5] = stock color modes 0..8 unchanged")
    for cls in range(4):
        fs = [f for f in fl.values() if f.paper_cls == cls]
        papers = range(10) if cls else (0, 3)
        bad = []
        for f, p in product(fs, papers):
            nv = {**zero, **({PAPER_NV[cls]: p} if cls else {})}
            if pack(f.cm, 0, **nv)[0] != packed_of(f, f.cm, nv):
                bad.append((f.cm, p))
        r.eq(bad, [], f"cmd[5] packed mode, paper class {cls}: {len(fs)} films x papers {papers[0]}..{papers[-1]}")
    f = next(iter(fl.values()))
    bad = []
    for e, g, h, fill, cs in product((0, 1), (0, 1), (0, 1), range(4), range(4)):
        got = pack(f.cm, cs, **{**zero, "engine": e, "grain": g, "halation": h, "fill": fill})[1]
        if got != cs | (e | g << 1 | h << 2) << 2 | fill << 5:
            bad.append((e, g, h, fill, cs, got))
    r.eq(bad, [], "cmd[6] = colorspace | (3d | grain<<1 | halation<<2) << 2 | fill << 5 (128 combos)")
    r.eq(regs_bad, [], "IPC hook keeps live r1 (1.0f) and returns r0 = 0")


def check_lv(b: Build, r: Report) -> None:
    for site, name in ((stock.LV_GAMMA_SITE, "lv_gamma_hook"), (stock.LV_MONO_SITE, "lv_mono_hook"), (stock.LV_CMCM_SITE, "lv_cmcm_hook")):
        r.eq(b.img.bl_target(site), b.sym(name), f"BL @ {site:#x} -> {name}")
    m = Machine(b.img)
    m.load_nv_defaults()
    calls: list[tuple] = []
    m.stub(b.sym("stock_b2y_set_gamma"), on=lambda a: calls.append(("gamma", a[1], m.rd(a[1], a[2]))))
    m.stub(b.sym("stock_lv_mono"), on=lambda a: calls.append(("mono", a[0], a[1])))
    m.stub(b.sym("stock_lv_cmcm"), on=lambda a: calls.append(("cmcm", a[0], a[1])))
    m.stub(b.sym("stock_b2y_set_mat"), on=lambda a: calls.append(("mat", struct.unpack(">14H", m.rd(a[0], 28)))))
    cc = {7: [96, -20, 5, -30, 120, 8, 2, -15, 110], 1: [130, -40, 10, -25, 140, -12, 0, -30, 150]}
    for mode, getter in ((7, "stock_get_lv_cc7"), (1, "stock_get_lv_ccn")):
        raw = bytes(10) + b"".join(struct.pack(">H", (v & 0xFFF) | 0xA000) for v in cc[mode]) + bytes(8)
        m.stub(b.sym(getter), on=lambda a, raw=raw: m.wr(a[0], raw))
    lk, fl = b.gen.looks, films(b)

    def look(f: Film) -> tuple[tuple[int, ...], bytes, int]:
        return lk[packed_of(f, f.cm, {}) - FILM_MODE0]

    def run(fn: str, *args: int) -> list[tuple]:
        calls.clear()
        m.call(b.sym(fn), *args)
        return list(calls)

    lut = SCRATCH + 0x1000
    stock_lut = bytes(range(256)) * 4
    m.wr(lut, stock_lut)
    gbuf = b.sym("gamma_buf")
    kf = next(f for f in fl if look(f)[2] & LV_KNOTS)
    curve = expand_knots(look(kf)[1])
    m.w8(stock.NV_CM, 0)
    r.eq(run("lv_gamma_hook", 0, lut, 0x400), [("gamma", lut, stock_lut)], "gamma hook, stock color mode: stock LUT")
    m.w8(stock.NV_CM, kf.cm)
    r.eq(run("lv_gamma_hook", 0, lut, 0x400), [("gamma", gbuf, curve)], f"gamma hook, film cm {kf.cm}: expanded film knots")
    m.w8(stock.NV_CM, 0)
    run("lv_gamma_hook", 0, lut, 0x400)
    mono_cm = 2 if kf.mono else 3
    r.eq(run("lv_mono_hook", 1, kf.cm), [("mono", 1, mono_cm), ("gamma", gbuf, curve)], f"color mode 0 -> {kf.cm}: mono routing + gamma re-applied")
    r.eq(run("lv_mono_hook", 1, kf.cm), [("mono", 1, mono_cm)], "same color mode: gamma not re-applied")
    r.eq(run("lv_mono_hook", 1, 0), [("mono", 1, 0), ("gamma", lut, stock_lut)], f"color mode {kf.cm} -> 0: stock gamma restored")

    r.eq(run("lv_cmcm_hook", 1, 0), [("cmcm", 1, 0)], "CMCM hook, stock color mode: stock path")
    cf = next(f for f in fl if look(f)[2] & LV_CMCM and not look(f)[2] & LV_MONO_Q7)
    for mode in (7, 1):
        exp = lv_matrix(look(cf)[0], look(cf)[2], cc[mode])
        r.eq(run("lv_cmcm_hook", mode, cf.cm), [("mat", exp)], f"CMCM hook, film cm {cf.cm}, LV mode {mode}: q10 film x live q7")
    mf = next((f for f in fl if look(f)[2] & LV_MONO_Q7), None)
    if mf:
        exp = lv_matrix(look(mf)[0], look(mf)[2], cc[1])
        r.eq(run("lv_cmcm_hook", 1, mf.cm), [("mat", exp)], f"CMCM hook, mono film cm {mf.cm}: direct q7 weights")


def check_ycap(b: Build, r: Report) -> None:
    m = Machine(b.img)
    lp = b.syms["lv_planes"]
    r.check(not stock.NV_BLOCK <= lp.va < stock.NV_BLOCK + stock.NV_LEN, f"lv_planes @ {lp.va:#x} outside the NVRAM block")
    site = stock.LV_YCAP + 2
    lit = b.stock.u32(((site + 4) & ~3) + (b.stock.u16(site) & 0xFF) * 4)
    planes = (0xA2300000, 0xA2360000, 0xA2390000)
    m.wr(SCRATCH, struct.pack(">3I", *planes))
    f0 = m.u32(lp.va + 12)
    with m.writes(STACK) as log:
        for _ in range(3):
            m.run(stock.LV_YCAP, stock.LV_YCAP + 6, r0=SCRATCH)
    got = (m.reg("r5"), m.reg("r7"), m.rd(lp.va, 12), m.u32(lp.va + 12))
    r.eq(got, (lit, SCRATCH, struct.pack(">3I", *planes), f0 + 3), "LV Y-capture hook: planes + frame count, stock r5/r7")
    r.eq(outside(log, (lp.va, lp.va + lp.size)), [], "LV Y-capture hook writes only lv_planes (never NVRAM)")


def check_updater(b: Build, r: Report) -> tuple[int, int]:
    maj, mi = (int(x) for x in b.pkg.version.split("."))
    r.eq(b.img.rd(stock.VERSION_IMM, 4), bytes([0x22, maj, 0x23, mi]), f"'Current version' immediates = {maj}.{mi:02}")
    for va in stock.UPDATER_BANNERS:
        old, new = b.stock.cstr(va), b.img.cstr(va)
        slot = (len(old) + 4) & ~3
        ok = "Debug Write" in old and bool(new) and new != old and b.img.rd(va, slot) == new.encode().ljust(slot, b"\0")
        r.check(ok and b.img.rd(va + slot, 4) == b.stock.rd(va + slot, 4), f"updater banner {va:#x}: {old!r} -> {new!r}")
    return maj, mi


def check_package(b: Build, r: Report, maj: int, mi: int) -> None:
    p, s = b.pkg, b.stock.pkg
    r.eq(b.pkg_name, f"DP2MV{maj}{mi:02}.bin", "package file name matches the version")
    r.check(len(set(p.versions)) == 1 and bool(re.fullmatch(r"\d+\.\d\d", p.version)), f"header versions {p.versions}")
    r.check(bool(re.fullmatch(r"20\d{6}", p.date)), f"header date {p.date}")
    r.eq((len(p.raw), p.flags, p.c78_off), (len(s.raw), s.flags, s.c78_off), "size, stage flags, M6M segment offset = stock")
    fields = {*range(0x10, 0x20), *range(0x30, 0x58)}
    r.eq([i for i in range(0x80) if i not in fields and p.raw[i] != s.raw[i]], [], "header bytes outside version/date = stock")
    r.check(p.segment("bridge") == s.segment("bridge"), "bridge M6M segment is stock")
    r.check(p.segment("m5") == (b.dir / "Firm.bin").read_bytes(), "M5 segment = Firm.bin")
    r.check(p.segment("m6m") == b.img.c78, "M6M segment = C78FRM.bin")


def run(b: Build, r: Report) -> None:
    check_invariants(b, r)
    check_menus(b, r)
    check_strings(b, r)
    addrs = check_settings(b, r)
    check_bracketing(b, r)
    check_min_shutter(b, r, addrs["minshutter"])
    check_ipc(b, r, addrs)
    check_lv(b, r)
    check_ycap(b, r)
    check_package(b, r, *check_updater(b, r))
