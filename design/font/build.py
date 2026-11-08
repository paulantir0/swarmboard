"""Build Swarm Mono (Regular 400 + SemiBold 600) from glyphs.py.

usage: python build.py [--out-web DIR] [--out-dist DIR]
requires: fonttools brotli shapely
"""
import argparse
import math
import os
import sys

from fontTools.fontBuilder import FontBuilder
from fontTools.misc.timeTools import timestampFromString
from fontTools.pens.ttGlyphPen import TTGlyphPen
from shapely.geometry import MultiPolygon, Polygon
from shapely.geometry.polygon import orient

sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
import glyphs as G  # noqa: E402

FAMILY = "Swarm Mono"
VERSION = "1.000"
WEIGHTS = {400: ("Regular", 76), 600: ("SemiBold", 104)}
CORNER_DEG = 24
STAMP = timestampFromString("Sun Nov  8 00:00:00 2026")


def _polys(geom):
    if geom.is_empty:
        return []
    if isinstance(geom, Polygon):
        return [geom]
    if isinstance(geom, MultiPolygon):
        return list(geom.geoms)
    return [g for g in getattr(geom, "geoms", []) if isinstance(g, Polygon) and not g.is_empty]


def _ring_to_pen(pen, coords):
    """Draw a closed ring. Smooth vertices become TrueType off-curve points, so
    densely sampled curves turn into quadratic splines; corners stay on-curve."""
    pts = [(round(x), round(y)) for x, y in coords[:-1]]
    dedup = []
    for p in pts:
        if not dedup or p != dedup[-1]:
            dedup.append(p)
    if len(dedup) > 1 and dedup[0] == dedup[-1]:
        dedup.pop()
    pts = dedup
    n = len(pts)
    if n < 3:
        return
    on = []
    for i in range(n):
        a, b, c = pts[i - 1], pts[i], pts[(i + 1) % n]
        v1 = (b[0] - a[0], b[1] - a[1])
        v2 = (c[0] - b[0], c[1] - b[1])
        l1, l2 = math.hypot(*v1), math.hypot(*v2)
        cosang = (v1[0] * v2[0] + v1[1] * v2[1]) / (l1 * l2)
        ang = math.degrees(math.acos(max(-1, min(1, cosang))))
        # corners, and points joining two long straight runs, stay on-curve
        on.append(ang > CORNER_DEG or (l1 > 60 and l2 > 60) or (ang > 8 and max(l1, l2) > 60))
    if not any(on):
        pen.qCurveTo(*pts, None)
        pen.closePath()
        return
    k = on.index(True)
    pts = pts[k:] + pts[:k]
    on = on[k:] + on[:k]
    pen.moveTo(pts[0])
    offs = []
    for p, is_on in zip(pts[1:], on[1:]):
        if is_on:
            if offs:
                pen.qCurveTo(*offs, p)
                offs = []
            else:
                pen.lineTo(p)
        else:
            offs.append(p)
    if offs:
        pen.qCurveTo(*offs, pts[0])
    pen.closePath()


def geom_to_glyph(geom):
    pen = TTGlyphPen(None)
    for poly in _polys(geom):
        poly = orient(poly.simplify(1.0, preserve_topology=True), sign=-1.0)  # TrueType: clockwise outer
        _ring_to_pen(pen, list(poly.exterior.coords))
        for hole in poly.interiors:
            _ring_to_pen(pen, list(hole.coords))
    return pen.glyph()


def glyph_name(ch):
    if ch == " ":
        return "space"
    cp = ord(ch)
    if ch.isascii() and ch.isalnum():
        return ch if not ch.isdigit() else ["zero", "one", "two", "three", "four", "five", "six", "seven", "eight", "nine"][int(ch)]
    return "uni%04X" % cp


def build(weight, out_paths):
    style, stroke = WEIGHTS[weight]
    ctx = G.Ctx(stroke)
    chars = sorted(G.GLYPHS, key=ord)
    order = [".notdef"] + [glyph_name(ch) for ch in chars]
    cmap = {ord(ch): glyph_name(ch) for ch in chars}

    glyf = {}
    nd = G.Glyph(60)
    nd.s([(100, 0), (500, 0), (500, G.CAP), (100, G.CAP), (100, 0)])
    glyf[".notdef"] = geom_to_glyph(nd.geometry())
    for ch in chars:
        glyf[glyph_name(ch)] = geom_to_glyph(G.GLYPHS[ch](ctx).geometry())

    fb = FontBuilder(1000, isTTF=True)
    fb.setupGlyphOrder(order)
    fb.setupCharacterMap(cmap)
    fb.setupGlyf(glyf)
    metrics = {}
    gt = fb.font["glyf"]
    for name in order:
        g = gt[name]
        g.recalcBounds(gt)
        metrics[name] = (G.ADV, getattr(g, "xMin", 0) if g.numberOfContours else 0)
    fb.setupHorizontalMetrics(metrics)
    fb.setupHorizontalHeader(ascent=G.LINE_TOP, descent=G.LINE_BOT, lineGap=0)
    ps_name = "SwarmMono-" + style
    fb.setupNameTable({
        "familyName": FAMILY if weight == 400 else f"{FAMILY} {style}",
        "styleName": "Regular",
        "typographicFamily": FAMILY,
        "typographicSubfamily": style,
        "uniqueFontIdentifier": f"{VERSION};SWRM;{ps_name}",
        "fullName": f"{FAMILY} {style}",
        "psName": ps_name,
        "version": f"Version {VERSION}",
        "copyright": "Copyright 2026 swarmboard",
        "designer": "swarmboard",
        "description": "Swarm Mono, an editorial monospace for swarmboard.",
    })
    fb.setupOS2(
        version=4,
        usWeightClass=weight,
        usWidthClass=5,
        xAvgCharWidth=G.ADV,
        sTypoAscender=G.LINE_TOP,
        sTypoDescender=G.LINE_BOT,
        sTypoLineGap=0,
        usWinAscent=G.LINE_TOP,
        usWinDescent=-G.LINE_BOT,
        sxHeight=G.XH,
        sCapHeight=G.CAP,
        fsSelection=(1 << 6) | (1 << 7),  # REGULAR | USE_TYPO_METRICS
        achVendID="SWRM",
        panose={"bFamilyType": 2, "bSerifStyle": 11, "bWeight": 6 if weight == 400 else 7, "bProportion": 9,
                "bContrast": 2, "bStrokeVariation": 2, "bArmStyle": 2, "bLetterForm": 2, "bMidline": 2, "bXHeight": 4},
        ulCodePageRange1=1,
    )
    fb.setupPost(isFixedPitch=1, underlinePosition=-150, underlineThickness=stroke)
    fb.setupHead(unitsPerEm=1000, fontRevision=float(VERSION), created=STAMP, modified=STAMP)
    fb.font["head"].macStyle = 0
    fb.font.recalcBBoxes = True
    from fontTools.ttLib import newTable
    gasp = newTable("gasp")
    gasp.gaspRange = {0xFFFF: 0x000F}
    fb.font["gasp"] = gasp

    base = f"swarm-mono-{weight}"
    for d, flavors in out_paths:
        os.makedirs(d, exist_ok=True)
        for flavor in flavors:
            fb.font.flavor = None if flavor == "ttf" else flavor
            p = os.path.join(d, f"{base}.{flavor}")
            fb.font.save(p)
            print("wrote", p, os.path.getsize(p), "bytes")


def main():
    here = os.path.dirname(os.path.abspath(__file__))
    ap = argparse.ArgumentParser()
    ap.add_argument("--out-web", default=os.path.join(here, "..", "..", "web", "public", "fonts"))
    ap.add_argument("--out-dist", default=os.path.join(here, "dist"))
    a = ap.parse_args()
    for w in WEIGHTS:
        build(w, [(a.out_dist, ["ttf"]), (a.out_web, ["woff2"])])


if __name__ == "__main__":
    main()
