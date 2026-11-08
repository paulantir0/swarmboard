"""Build the swarmboard mark, wordmark and favicons.

usage: python build.py [--previews DIR]
requires: fonttools shapely pillow; run design/font/build.py first.
"""
import argparse
import math
import os
import sys

from fontTools.pens.svgPathPen import SVGPathPen
from fontTools.pens.transformPen import TransformPen
from fontTools.ttLib import TTFont
from PIL import Image, ImageDraw
from shapely.geometry import LineString, box
from shapely.ops import unary_union

HERE = os.path.dirname(os.path.abspath(__file__))
ROOT = os.path.join(HERE, "..", "..")
OUT = os.path.join(ROOT, "web", "public", "brand")
FONT = os.path.join(ROOT, "design", "font", "dist", "swarm-mono-600.ttf")

S = 64  # mark grid
INK, PAPER = "#111111", "#fafafa"
INK_DARK, CANVAS_DARK = "#f2eee6", "#11110f"


def wave(x0, x1, y, amp, w, period=27.0, phase_x=15.5):
    """Sine stroke; phase is tied to absolute x so staggered waves stay parallel."""
    n = 48
    pts = [(x, y + amp * math.sin(2 * math.pi * (x - phase_x) / period))
           for x in (x0 + (x1 - x0) * i / n for i in range(n + 1))]
    return LineString(pts).buffer(w / 2, cap_style="round", join_style="round", quad_segs=8)


def mark(stroke=5.5, wave_w=5.5):
    """A board (square frame) holding three staggered waves: posts on a board, water in motion."""
    m = 4
    frame = box(m, m, S - m, S - m).difference(box(m + stroke, m + stroke, S - m - stroke, S - m - stroke))
    waves = [wave(15.5, 42, 43, 2.7, wave_w), wave(22, 48.5, 32, 2.7, wave_w), wave(15.5, 42, 21, 2.7, wave_w)]
    return unary_union([frame] + waves)


def polys(g):
    return [g] if g.geom_type == "Polygon" else list(g.geoms)


def geom_to_d(g, tol=0.04):
    """SVG path data, y flipped (grid is y-up)."""
    out = []
    for p in polys(g.simplify(tol)):
        for ring in [p.exterior] + list(p.interiors):
            cs = list(ring.coords)[:-1]
            out.append("M" + "L".join(f"{x:.2f} {S - y:.2f}".replace(".00", "") for x, y in cs) + "Z")
    return "".join(out)


def wordmark_d(text, x0, baseline, size):
    font = TTFont(FONT)
    gs = font.getGlyphSet()
    cmap = font.getBestCmap()
    scale = size / font["head"].unitsPerEm
    pen = SVGPathPen(gs, ntos=lambda v: f"{v:.1f}".rstrip("0").rstrip("."))
    x = x0
    for ch in text:
        name = cmap[ord(ch)]
        gs[name].draw(TransformPen(pen, (scale, 0, 0, -scale, x, baseline)))
        x += gs[name].width * scale
    return pen.getCommands(), x


def raster(g, px, fg, bg):
    k = 4
    mask = Image.new("L", (px * k, px * k), 0)
    sc = px * k / S
    for p in polys(g):
        layer = Image.new("L", mask.size, 0)
        d = ImageDraw.Draw(layer)
        T = lambda cs: [(x * sc, (S - y) * sc) for x, y in cs]
        d.polygon(T(p.exterior.coords), fill=255)
        for h in p.interiors:
            d.polygon(T(h.coords), fill=0)
        mask = Image.composite(layer, mask, layer)
    mask = mask.resize((px, px), Image.LANCZOS)
    out = Image.new("RGB", (px, px), bg)
    out.paste(Image.new("RGB", (px, px), fg), mask=mask)
    return out


def write(name, text):
    p = os.path.join(OUT, name)
    with open(p, "w") as f:
        f.write(text)
    print("wrote", p, len(text), "bytes")


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument("--previews", default=None)
    a = ap.parse_args()
    os.makedirs(OUT, exist_ok=True)

    g = mark()
    d = geom_to_d(g)
    write("mark.svg", f'<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 {S} {S}" fill="currentColor">'
                      f'<title>swarmboard</title><path fill-rule="evenodd" d="{d}"/></svg>\n')

    fav = mark(stroke=6.5, wave_w=6.5)
    write("favicon.svg",
          f'<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 {S} {S}">'
          f'<style>path{{fill:{INK}}}@media (prefers-color-scheme:dark){{path{{fill:{INK_DARK}}}}}</style>'
          f'<path fill-rule="evenodd" d="{geom_to_d(fav)}"/></svg>\n')

    # wordmark: mark at cap-ish height + lowercase text; mark height = 1.0 * x-height box of text
    size = 64  # font size in wordmark units
    gap = 18
    text_d, x_end = wordmark_d("swarmboard", S + gap, 0, size)
    # place text so its x-height (0.52em) is vertically centred on the mark
    xh = 0.52 * size
    baseline = S / 2 + xh / 2
    text_d, x_end = wordmark_d("swarmboard", S + gap, baseline, size)
    W = math.ceil(x_end)
    write("wordmark.svg",
          f'<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 {W} {S}" fill="currentColor">'
          f'<title>swarmboard</title><path fill-rule="evenodd" d="{d}"/><path d="{text_d}"/></svg>\n')

    touch = Image.new("RGB", (180, 180), PAPER)
    touch.paste(raster(mark(stroke=5.5), 120, INK, PAPER), (30, 30))
    touch.save(os.path.join(OUT, "apple-touch-icon.png"))
    print("wrote apple-touch-icon.png")

    if a.previews:
        from PIL import ImageFont
        os.makedirs(a.previews, exist_ok=True)
        for theme, fg, bg in (("light", INK, PAPER), ("dark", INK_DARK, CANVAS_DARK)):
            im = Image.new("RGB", (1200, 560), bg)
            dr = ImageDraw.Draw(im)
            im.paste(raster(g, 160, fg, bg), (60, 60))
            # wordmark: mark 96px + text
            m = 96
            im.paste(raster(g, m, fg, bg), (60, 300))
            f = ImageFont.truetype(FONT, m)
            # baseline so x-height centred on mark: PIL draws from ascender (0.93em) top
            base = 300 + m / 2 + 0.52 * m / 2
            dr.text((60 + m + gap * m / S, base - 0.93 * m), "swarmboard", font=f, fill=fg)
            for i, px in enumerate((16, 24, 32, 48, 64)):
                im.paste(raster(fav if px <= 32 else g, px, fg, bg), (300 + i * 100, 100))
            im.save(os.path.join(a.previews, f"logo-{theme}.png"))
        Image.open(os.path.join(OUT, "apple-touch-icon.png")).save(os.path.join(a.previews, "apple-touch-icon.png"))


if __name__ == "__main__":
    main()
