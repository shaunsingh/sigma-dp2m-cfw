"""UI tables: strings by sid, menus via Get_Menu_Table, screen vtable, icon table & contact sheets."""

from __future__ import annotations

from collections.abc import Iterable
from dataclasses import dataclass
from pathlib import Path

import numpy as np
from PIL import Image as PImage
from PIL import ImageDraw

from . import stock
from .emu import EmuError, Machine
from .image import BASE, CODE_END, END, Image


@dataclass(frozen=True)
class Row:
    """32 B menu row."""

    va: int
    w: tuple[int, ...]

    id = property(lambda s: s.w[0])  # submenu id for page rows
    qs_icon = property(lambda s: s.w[1] & 0xFFFF)  # 110x70 QS value
    strip_icon = property(lambda s: s.w[2] & 0xFFFF)  # 54x42 QS strip / main menu value
    osd_icon = property(lambda s: s.w[2] >> 16)  # 76x26
    sid = property(lambda s: s.w[3] >> 16)
    value = property(lambda s: s.w[4])
    get = property(lambda s: s.w[5])
    set = property(lambda s: s.w[6])
    extra = property(lambda s: s.w[7])


def rows(img: Image | Machine, va: int, limit: int = 128) -> list[Row]:
    out = []
    for k in range(limit):
        w = tuple(int.from_bytes(img.rd(va + 32 * k + 4 * i, 4), "big") for i in range(8))
        if not any(w):
            break
        out.append(Row(va + 32 * k, w))
    return out


def sid_text(img: Image, sid: int, lang: int = 0) -> str | None:
    """Stock string table entry (no payload hooks)."""
    if not 0 <= sid < stock.STOCK_SIDS:
        return None
    p = img.u32(stock.STR_TBL + sid * stock.STR_STRIDE + lang * 4)
    return img.cstr(p) if BASE <= p < END else None


def string(m: Machine, sid: int, lang: int = 0) -> str | None:
    """UI string lookup through the (possibly hooked) firmware function."""
    p = m.call(stock.UI_STRING, sid, lang, count=100_000)
    return m.cstr(p) if BASE <= p < 0xB0000000 else None


def menu_table(m: Machine, mid: int) -> int:
    return m.call(stock.GET_MENU_TABLE, mid, count=100_000)


def menu_tables(m: Machine, ids: Iterable[int] = range(0x200)) -> dict[int, int]:
    """id -> rows for every id Get_Menu_Table resolves."""
    out = {}
    for mid in ids:
        try:
            t = menu_table(m, mid)
        except EmuError:
            continue
        if BASE <= t < END and any(m.rd(t, 32)):
            out[mid] = t
    return out


def screens(img: Image) -> list[tuple[int, int]]:
    """(posted id, handler) of the screen vtable (ids from 1), up to the first non-code entry."""
    out = []
    for i in range(1, 256):
        h = img.u32(stock.SCREEN_VT + 4 * i)
        if not BASE <= h & ~1 < CODE_END:
            break
        out.append((i, h))
    return out


@dataclass(frozen=True)
class Icon:
    id: int
    bits: int
    w: int
    h: int

    def pixels(self, img: Image | Machine) -> np.ndarray:
        return np.frombuffer(img.rd(self.bits, self.w * self.h), np.uint8).reshape(self.h, self.w)


def _arm_ret(img: Image, fn: int) -> int | None:
    """Constant returned by an ARM getter `ldr r0,[pc,#imm]` / `mov r0,#imm`."""
    w = img.u32(fn)
    if w & 0xFFFFF000 == 0xE59F0000:
        return img.u32(fn + 8 + (w & 0xFFF))
    if w & 0xFFFFF000 == 0xE3A00000:
        rot, imm = ((w >> 8) & 0xF) * 2, w & 0xFF
        return ((imm >> rot) | (imm << (32 - rot))) & 0xFFFFFFFF
    return None


def icons(img: Image) -> dict[int, Icon]:
    """Stock icon table ids 1..ICON_MAX (12 B entries of ARM getters for bits, w, h)."""
    out = {}
    for i in range(1, stock.ICON_MAX + 1):
        bits, w, h = (_arm_ret(img, f & ~1) for f in img.words(stock.ICON_TBL + 12 * i, 3))
        if bits and w and h and BASE <= bits and bits + w * h <= END:
            out[i] = Icon(i, bits, w, h)
    return out


def palette(img: Image) -> np.ndarray:
    """OSD CLUT as RGB (CLUT_N x 3)."""
    ayuv = np.frombuffer(img.rd(stock.CLUT, 4 * stock.CLUT_N), np.uint8).reshape(-1, 4).astype(np.float64)
    y, cb, cr = ayuv[:, 1], ayuv[:, 2] - 128, ayuv[:, 3] - 128
    rgb = np.stack([y + 1.402 * cr, y - 0.344 * cb - 0.714 * cr, y + 1.772 * cb], axis=1)
    return np.clip(rgb, 0, 255).astype(np.uint8)


def to_rgb(px: np.ndarray, pal: np.ndarray, bg: tuple[int, int, int] = (52, 58, 70)) -> np.ndarray:
    """Icon / text coverage (1 white .. 16 black, 0 transparent) as drawn by an enabled row."""
    out = np.empty(px.shape + (3,), np.uint8)
    out[:] = bg
    for v in np.unique(px[px > 0]):
        out[px == v] = pal[stock.ICON_HI + 1 - v] if v <= 16 else pal[min(int(v), len(pal) - 1)]
    return out


def contact_sheet(tiles: list[tuple[str, np.ndarray]], path: Path, pal: np.ndarray, cols: int = 8, scale: int = 2,
                  bg: tuple[int, int, int] = (52, 58, 70), cap: tuple[int, int] = (240, 160)) -> None:
    """PNG grid of labelled coverage tiles; tiles larger than `cap` (after scaling) are shrunk."""
    cw = min(cap[0], max(64, max(t.shape[1] for _, t in tiles) * scale)) + 8
    ch = min(cap[1], max(t.shape[0] for _, t in tiles) * scale) + 20
    n_rows = (len(tiles) + cols - 1) // cols
    sheet = PImage.new("RGB", (cols * cw, n_rows * ch), (24, 24, 24))
    d = ImageDraw.Draw(sheet)
    for k, (label, t) in enumerate(tiles):
        x, y = (k % cols) * cw, (k // cols) * ch
        im = PImage.fromarray(to_rgb(t, pal, bg)).resize((t.shape[1] * scale, t.shape[0] * scale), PImage.NEAREST)
        im.thumbnail((cw - 8, ch - 20))
        sheet.paste(im, (x + 4, y + 16))
        d.text((x + 4, y + 2), label[: cw // 6], fill=(255, 220, 0))
    sheet.save(path)
