"""labels suite: runtime-rendered label icons, QS strip routing & LV color-mode icons (skips without the labels payload)."""

from __future__ import annotations

import random
import struct

import numpy as np

from tools.re import stock, ui
from tools.re.emu import STACK, SCRATCH, Machine

from .harness import Build, Report

DIM = {"L": (110, 70), "S": (54, 42), "OSD": (76, 26), "MODE": (51, 31), "WIDGET": (35, 20), "QS": (42, 34)}  # stock tile size per class
STOCK_IDS = {"L": [0x10E, 0x10F, 0x115, 0x113], "S": [0x159, 0x15A, 0x160], "OSD": [0x27, 0x28, 0x2E], "MODE": [0xB4, 0xB6], "WIDGET": [0xB7, 0xB9],
             "QS": [0xA1, 0x9B]}
NAMES = ["Velvia 100", "FOV Classic Yellow", "Kodak Tri-X 400", "Teal & Orange", "Cinema", "Portra 800 +2", "Mono Orange #21",
         "SD Custom LUT", "Rollei Retro 80S", "Fujicolor C200", "Zebras (95% IRE)", "Fill -0.5 (Contrast)"]
QS_ITEMS = ["Print Paper", "Film Engine", "Film Grain", "Halation", "X3 Fill Light"]
TOKENS = {"Red": "F1", "Yellow": "F2", "White": "M2", "Black": "P"}  # short test strings for the MODE / WIDGET classes
TOKEN_VA = SCRATCH + 0x10000
GET_CM, GET_ISO = stock.GET_CM | 1, 0xA016F5AF
STATE = stock.QS_STATE
LABELS_BOOT_SITE = 0xA01689BE  # boot call the label cache reset hooks
LABEL_MAGIC = 0x4C41424C  # "LABL", first word of the label cache


def run(b: Build, r: Report) -> None:
    b.require("label_icon", "cache", "arena", "lv_status_cm")
    for site, sym in ((stock.ICON_GET + 2, "icon_hook"), (stock.QS_STRIP_CHECK, "qs_strip_hook")):
        if sym in b.syms:
            r.eq(b.img.bl_target(site), b.sym(sym), f"BL @ {site:#x} -> {sym}")
    m = Machine(b.img)
    cls = {k: b.cat[f"ICON_LABEL_{k}"] for k in DIM}
    arena = b.syms["arena"]
    a0, a1 = arena.va, arena.va + arena.size
    bss = (b.sym("cache"), max(s.va + s.size for s in b.syms.values() if 0xA1C00000 <= s.va < 0xA1E00000))
    random.seed(1)
    m.wr(bss[0], bytes(random.getrandbits(8) for _ in range(0x6000)))  # payload RAM is never zeroed on camera

    table = b.sym("dp2m_strings")
    strings = list(b.gen.strings)
    for k, (orig, tok) in enumerate(TOKENS.items()):
        i = strings.index(orig)
        m.wr(TOKEN_VA + 0x40 * k, tok.encode() + b"\0")
        m.w32(table + 4 * i, TOKEN_VA + 0x40 * k)
        strings[i] = tok

    def icon(i: int) -> tuple[int, int, np.ndarray] | None:
        res = m.call(stock.ICON_GET, i, count=100_000_000)
        if not res:
            return None
        p, w, h = struct.unpack(">IHH", m.rd(res, 8))
        return res, p, np.frombuffer(m.rd(p, w * h), np.uint8).reshape(h, w).copy()

    allowed = (STACK, bss, (stock.LINE_BUF, stock.LINE_BUF + 0x28000), (stock.ICON_RESULT, stock.ICON_RESULT + 8),
               (stock.FONT_SCALE, stock.FONT_SCALE + 8), (stock.OSD_DIRTY, stock.OSD_DIRTY + 8), stock.GLYPH_RASTER, stock.FONT_WORK)
    sheets: list[tuple[str, np.ndarray]] = []
    with m.writes(*allowed) as log:
        bad = [i for ids in STOCK_IDS.values() for i in ids if (x := icon(i)) is None or x[0] != stock.ICON_RESULT]
        r.eq(bad, [], "stock icon ids pass through the hooked getter")
        m.wr(stock.FONT_SCALE, struct.pack(">II", 5, 7))  # a screen's width scale must not leak into tiles
        dirty0 = m.rd(stock.OSD_DIRTY, 16)
        for c, base in cls.items():
            sheets += [(f"stock {i:#x}", icon(i)[2]) for i in STOCK_IDS[c]]
            for name in (list(TOKENS.values()) + ["Cinema"]) if c in ("MODE", "WIDGET") else QS_ITEMS if c == "QS" else NAMES:
                i = base + strings.index(name)
                x, x2 = icon(i), icon(i)
                if not r.check(x is not None, f"{c} {name!r} ({i:#x}) renders"):
                    continue
                res, p, t = x
                w, h = DIM[c]
                vals = set(np.unique(t).tolist())
                fill = (t > 0) & (t <= 12)
                pad = np.pad(t, 1)
                gap = sum(int((fill & (pad[1 + dy : 1 + dy + h, 1 + dx : 1 + dx + w] == 0)).sum()) for dy, dx in ((0, 1), (0, -1), (1, 0), (-1, 0)))
                ok = (res == stock.ICON_RESULT and t.shape == (h, w) and a0 <= p and p + w * h <= a1 and x2[1] == p
                      and vals <= set(range(17)) and {1, 16} <= vals and gap == 0)
                r.check(ok, f"{c} {name!r}: {w}x{h} tile in arena, cached, levels 0..16 with fill & outline, outline gaps {gap}")
                sheets.append((name, t))
        r.eq(struct.unpack(">II", m.rd(stock.FONT_SCALE, 8)), (5, 7), "font scale restored")
        r.check(m.rd(stock.OSD_DIRTY, 16) == dirty0, "OSD dirty rects unchanged")
    r.eq(sorted({hex(a) for a, _ in log})[:8], [], "label rendering writes only allowed ranges")

    def onscreen(mm: Machine) -> bytes:
        mm.wr(stock.FONT_SCALE, struct.pack(">II", 1, 1))
        mm.wr(stock.OSD, bytes(640 * 480))
        mm.text("ISO 100 F2.8", 24, rect=(20, 200, 600, 60))
        return mm.rd(stock.OSD, 640 * 480)

    after, fresh = onscreen(m), onscreen(Machine(b.img))
    r.check(after == fresh and any(after), "stock text drawn after label renders matches a fresh camera")
    if "labels_boot" in b.syms:
        r.eq(b.img.bl_target(LABELS_BOOT_SITE), b.sym("labels_boot"), f"BL @ {LABELS_BOOT_SITE:#x} -> labels_boot")
        cache = b.sym("cache")
        m.stub(b.stock.bl_target(LABELS_BOOT_SITE))
        m.wr(cache, struct.pack(">II", LABEL_MAGIC, 0x1234) + struct.pack(">I", 0xDEADBEEF) * 16)
        m.call(b.sym("labels_boot"))
        c = struct.unpack(">18I", m.rd(cache, 72))
        r.check(c[0] == LABEL_MAGIC and not any(c[1:]), "labels_boot resets a cache that kept its magic over a power cycle")
    with m.writes(*allowed) as log:
        m.w32(b.sym("cache") + 4, arena.size - 100)
        x = icon(cls["L"] + strings.index("Velvia 100"))
        r.check(x is not None and x[1] == a0, "arena full -> reset & re-render at the arena base")
        bad = [i for i in (0x1C00 + 1, cls["L"] + len(strings), 0x1FFF) if m.call(stock.ICON_GET, i)]
        r.eq(bad, [], "invalid label ids return NULL")
    r.eq(sorted({hex(a) for a, _ in log})[:8], [], "arena reset writes only allowed ranges")
    if r.artifacts:
        ui.contact_sheet(sheets, r.artifacts / "labels.png", ui.palette(b.img), cols=12, scale=2)
    check_ui(b, r, m, strings)


def check_ui(b: Build, r: Report, m: Machine, strings: list[str]) -> None:
    """QS strip routing for the color mode slot and LV color-mode icons."""
    m.load_nv_defaults()
    m.stub(stock.STRIP_BG_CLEAR)
    arena = b.syms["arena"]

    def in_arena(p: int) -> bool:
        return arena.va <= p < arena.va + arena.size

    blits: list[tuple[int, ...]] = []
    m.watch(stock.BLIT, lambda a: blits.append((*a, *m.stack_args(3))))  # src, x, y, w, h, hi, lo
    slots = {}
    for page, s in ((p, s) for p in (0, 1) for s in range(4)):
        if rows := m.call(stock.QS_SLOT_ROWS, page, s):
            slots[(page, s)] = (rows, m.u32(rows + 0x14))
    cm_slot = next((k for k, v in slots.items() if v[1] == GET_CM), None)
    if not r.check(cm_slot is not None, f"QS color mode slot found in {len(slots)} slots"):
        return
    rows = slots[cm_slot][0]
    n_rows = m.call(stock.MENU_ROW_COUNT, rows)
    values = [m.u32(rows + 32 * i + 0x10) for i in range(n_rows)]

    def strip(slot: tuple[int, int], cm: int) -> tuple[int, list]:
        page, s = slot
        m.w16(STATE + 4, page)
        m.w16(STATE + 8 + 2 * page, s)
        m.w8(stock.NV_CM, cm)
        m.wr(stock.OSD, bytes(640 * 480))
        blits.clear()
        m.call(stock.QS_STRIP, count=200_000_000)
        return m.u16(STATE + 6), list(blits)

    m.w16(STATE + 6, 0)
    for sel in sorted({0, 7, 8, values.index(19) if 19 in values else 12, n_rows - 1}):
        scroll, bl = strip(cm_slot, values[sel])
        tiles = [x for x in bl if x[3] == 54]
        r.check(scroll <= sel <= scroll + 7 and len(tiles) == 8, f"QS strip cm {values[sel]} (row {sel}/{n_rows}): scroll {scroll}, {len(tiles)} tiles")
    other = next((k for k, v in slots.items() if v[1] not in (GET_CM, GET_ISO)), None)
    if other:
        m.w16(STATE + 6, 0)
        _, bl = strip(other, values[0])
        r.check(len(bl) > 0, f"other QS slot {other} keeps the generic strip ({len(bl)} blits)")

    def film(name: str) -> int:
        return 9 + strings.index(name)

    for cm in (4, 2, film("Velvia 100"), film("Kodak Tri-X 400"), film("FOV Classic Yellow"), film("Teal & Orange")):
        m.w8(stock.NV_CM, cm)
        for name, fn, args in (("LV status", b.sym("lv_status_cm"), (cm,)), ("LV info", stock.LV_INFO_CM, ())):
            blits.clear()
            m.call(fn, *args, count=100_000_000)
            x = blits[-1] if blits else None
            ok = x is not None and in_arena(x[0]) == (cm >= 9) and (x[3], x[4]) == (76, 26)
            r.check(ok, f"{name} icon cm {cm}: {'label tile' if cm >= 9 else 'stock icon'} 76x26")
    for cm in (4, film("Velvia 100")):
        m.w8(stock.NV_CM, cm)
        i = m.call(stock.ROW_OSD_ICON, b.sym("m_f7"), 0)
        res = m.call(stock.ICON_GET, i)
        p, w, h = struct.unpack(">IHH", m.rd(res, 8)) if res else (0, 0, 0)
        exp = b.cat["ICON_LABEL_OSD"] + cm - 9 if cm >= 9 else 0x28
        r.check(i == exp and (w, h) == (76, 26) and in_arena(p) == (cm >= 9), f"Film page row 0 OSD icon cm {cm}: {i:#x} -> {w}x{h}")
