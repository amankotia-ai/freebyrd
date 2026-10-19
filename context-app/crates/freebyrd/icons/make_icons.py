"""Draws Freebyrd's icons from the falcon mark (freebyrd-mark.svg).

    python3 -I crates/freebyrd/icons/make_icons.py

Writes the app icon (icons/icon.png, and icons/freebyrd-icon.svg for pages
such as the case study), the menu bar's template image (icons/tray.png), the
iPad app's icons (companion-ui/icon-*.png) and the extension's
(extension/icons/). Needs Pillow and numpy only.
"""

import re
from pathlib import Path

import numpy as np
from PIL import Image, ImageDraw

HERE = Path(__file__).resolve().parent
CRATE = HERE.parent
ROOT = CRATE.parent.parent

# The mark in a 1000-wide box, traced from the falcon drawing: a flat top
# that ends in the beak, the body hanging below it, a tail tapering left.
MARK = (
    "M0 0H828.21C908.13 0 998.69 0 1000 66.04C984.96 66.89 971.55 60.26 944.05 68.63"
    "C881.83 87.56 847.76 153.45 745.43 153.45C649.33 153.45 607.14 122.04 524.74 88.79"
    "C362.81 23.46 172.71 13.58 0 13.16Z"
)
MARK_W, MARK_H = 1000.0, 153.45

# The notch's monochrome: a graphite tile, the mark in Apple's off-white.
TILE_TOP, TILE_BOTTOM = (44, 44, 46), (10, 10, 11)
MARK_RGB = (245, 245, 247)
SS = 8  # supersampling


def mark_points(steps=160):
    """The mark's outline as points in its 1000-wide box."""
    tokens = re.findall(r"[MHCZ]|-?\d+(?:\.\d+)?", MARK)
    pts, i, cur = [], 0, (0.0, 0.0)
    while i < len(tokens):
        cmd = tokens[i]
        i += 1
        if cmd == "M":
            cur = (float(tokens[i]), float(tokens[i + 1]))
            i += 2
            pts.append(cur)
        elif cmd == "H":
            cur = (float(tokens[i]), cur[1])
            i += 1
            pts.append(cur)
        elif cmd == "C":
            while i < len(tokens) and tokens[i] not in "MHCZ":
                p = np.array([cur] + [(float(tokens[i + k]), float(tokens[i + k + 1])) for k in (0, 2, 4)])
                i += 6
                t = np.linspace(0, 1, steps)[1:, None]
                q = (1 - t) ** 3 * p[0] + 3 * (1 - t) ** 2 * t * p[1] + 3 * (1 - t) * t**2 * p[2] + t**3 * p[3]
                pts += [tuple(v) for v in q]
                cur = tuple(p[3])
    return pts


def mark_mask(size, left, top, width, grow=0.0):
    """An antialiased mask of the mark, `width` px wide at (left, top).

    `grow` thickens it by that many px on every side, so the tail survives
    at menu bar size."""
    w, h = size
    big = Image.new("L", (w * SS, h * SS), 0)
    d = ImageDraw.Draw(big)
    k = width / MARK_W
    pts = [((left + x * k) * SS, (top + y * k) * SS) for x, y in mark_points()]
    d.polygon(pts, fill=255)
    if grow > 0:
        d.line(pts + [pts[0]], fill=255, width=max(1, round(2 * grow * SS)), joint="curve")
    return big.resize((w, h), Image.LANCZOS)


def squircle_mask(size, left, top, side, n=5.0):
    """A superellipse, close to the continuous corners of macOS icons."""
    w, h = size
    ys, xs = np.mgrid[0 : h * SS, 0 : w * SS].astype(np.float32)
    cx, cy, r = (left + side / 2) * SS, (top + side / 2) * SS, side / 2 * SS
    inside = (np.abs((xs + 0.5 - cx) / r) ** n + np.abs((ys + 0.5 - cy) / r) ** n) <= 1
    return Image.fromarray((inside * 255).astype(np.uint8)).resize((w, h), Image.LANCZOS)


def gradient(size, top, bottom):
    w, h = size
    t = np.linspace(0, 1, h)[:, None, None]
    rgb = np.array(top) * (1 - t) + np.array(bottom) * t
    return Image.fromarray(np.repeat(rgb, w, axis=1).astype(np.uint8), "RGB").convert("RGBA")


def tile(size, rounded, mark_share=0.66):
    """The icon: graphite tile with the mark centred.

    rounded: a macOS squircle on the 1024 grid (824 body, room for the
    shadow); otherwise full bleed, for iPadOS to round itself."""
    s = size
    if rounded:
        side = s * 824 / 1024
        left = top = (s - side) / 2
    else:
        side, left, top = s, 0.0, 0.0
    img = Image.new("RGBA", (s, s), (0, 0, 0, 0))
    body = gradient((s, s), TILE_TOP, TILE_BOTTOM)
    if rounded:
        shape = squircle_mask((s, s), left, top, side)
        # Apple's template shadow: 10 px down, 10 px blur, black at 30%, on the 1024 grid.
        from PIL import ImageFilter

        shadow = Image.new("RGBA", (s, s), (0, 0, 0, 0))
        offset = squircle_mask((s, s), left, top + s * 10 / 1024, side)
        shadow.putalpha(offset.point(lambda v: v * 0.3))
        img = Image.alpha_composite(img, shadow.filter(ImageFilter.GaussianBlur(s * 10 / 1024 / 2)))
        # A faint light rim so the tile holds on a dark Dock or page.
        inner = squircle_mask((s, s), left + s * 2 / 1024, top + s * 2 / 1024, side - s * 4 / 1024)
        rim = Image.new("RGBA", (s, s), (255, 255, 255, 0))
        rim.putalpha(Image.fromarray((np.asarray(shape, np.float32) - np.asarray(inner, np.float32)).clip(0, 255).astype(np.uint8)).point(lambda v: v * 0.12))
        body.putalpha(shape)
        img = Image.alpha_composite(img, body)
        img = Image.alpha_composite(img, rim)
    else:
        img = body
    mw = side * mark_share
    mh = mw * MARK_H / MARK_W
    mask = mark_mask((s, s), left + (side - mw) / 2, top + (side - mh) / 2, mw)
    ink = Image.new("RGBA", (s, s), MARK_RGB + (0,))
    ink.putalpha(mask)
    return Image.alpha_composite(img, ink)


def tray(height_px=36, width_pt=30, grow=0.45):
    """The menu bar's template image: black on clear, 18 pt tall at 2x.

    macOS tints a template for light and dark menu bars. tray-icon scales
    the image to 18 pt tall, so the canvas sets the width."""
    scale = height_px / 18
    w = round(width_pt * scale)
    mw = w - 2 * grow - 2
    mh = mw * MARK_H / MARK_W
    mask = mark_mask((w, height_px), (w - mw) / 2, (height_px - mh) / 2, mw, grow=grow)
    img = Image.new("RGBA", (w, height_px), (0, 0, 0, 0))
    img.putalpha(mask)
    return img


def squircle_path(side, inset=0.0, n=5.0, steps=256):
    r = side / 2 - inset
    pts = []
    for k in range(steps):
        a = 2 * np.pi * k / steps
        c, s = np.cos(a), np.sin(a)
        pts.append((side / 2 + r * np.sign(c) * abs(c) ** (2 / n), side / 2 + r * np.sign(s) * abs(s) ** (2 / n)))
    return "M" + "L".join(f"{x:.2f} {y:.2f}" for x, y in pts) + "Z"


def tile_svg(mark_share=0.66):
    """The app icon's tile as a vector, for pages (the case study): the
    PNGs' tile, gradient, rim and mark, without the shadow."""
    side = 824
    mw = side * mark_share
    k = mw / MARK_W
    x, y = (side - mw) / 2, (side - MARK_H * k) / 2
    hexes = ["#%02x%02x%02x" % c for c in (TILE_TOP, TILE_BOTTOM, MARK_RGB)]
    return (
        f'<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 {side} {side}">'
        f'<defs><linearGradient id="g" x1="0" y1="0" x2="0" y2="1">'
        f'<stop offset="0" stop-color="{hexes[0]}"/><stop offset="1" stop-color="{hexes[1]}"/></linearGradient></defs>'
        f'<path fill="url(#g)" d="{squircle_path(side)}"/>'
        f'<path fill="#fff" fill-opacity=".12" fill-rule="evenodd" d="{squircle_path(side)}{squircle_path(side, 2)}"/>'
        f'<path fill="{hexes[2]}" transform="translate({x:.2f} {y:.2f}) scale({k:.5f})" d="{MARK}"/></svg>\n'
    )


def main():
    (HERE / "freebyrd-mark.svg").write_text(
        f'<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 {MARK_W:g} {MARK_H:g}"><path d="{MARK}"/></svg>\n'
    )
    (HERE / "freebyrd-icon.svg").write_text(tile_svg())
    big = tile(1024, rounded=True)
    big.resize((512, 512), Image.LANCZOS).save(HERE / "icon.png")
    tray().save(HERE / "tray.png")
    ipad = tile(1024, rounded=False)
    for n in (180, 192, 512):
        ipad.resize((n, n), Image.LANCZOS).save(CRATE / "companion-ui" / f"icon-{n}.png")
    ext = ROOT / "extension" / "icons"
    ext.mkdir(exist_ok=True)
    for n in (16, 32, 48, 128):
        # Chrome's toolbar wants a little room round the tile; the mark gets more of it when small.
        share = 0.78 if n <= 32 else 0.7
        tile(n * 4, rounded=True, mark_share=share).resize((n, n), Image.LANCZOS).save(ext / f"icon-{n}.png")


if __name__ == "__main__":
    main()
