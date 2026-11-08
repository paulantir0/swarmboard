"""Small geometry toolkit: centerline paths -> expanded, overlap-free outlines."""
import math

from shapely import affinity
from shapely.geometry import LineString, Point, Polygon, box
from shapely.ops import unary_union


def _seg(n, a, b):
    return [a + (b - a) * i / n for i in range(n + 1)]


def se(cx, cy, rx, ry, a0, a1, n=2.35, step=2.0):
    """Superellipse arc from angle a0 to a1 (degrees, either direction)."""
    steps = max(2, int(abs(a1 - a0) / step))
    pts = []
    for a in _seg(steps, a0, a1):
        t = math.radians(a)
        c, s = math.cos(t), math.sin(t)
        x = cx + rx * math.copysign(abs(c) ** (2 / n), c)
        y = cy + ry * math.copysign(abs(s) ** (2 / n), s)
        pts.append((x, y))
    return pts


def ell(cx, cy, rx, ry, a0, a1, step=2.0):
    return se(cx, cy, rx, ry, a0, a1, n=2.0, step=step)


def bez(p0, p1, p2, p3, n=40):
    out = []
    for i in range(n + 1):
        t = i / n
        u = 1 - t
        out.append((u**3 * p0[0] + 3 * u * u * t * p1[0] + 3 * u * t * t * p2[0] + t**3 * p3[0],
                    u**3 * p0[1] + 3 * u * u * t * p1[1] + 3 * u * t * t * p2[1] + t**3 * p3[1]))
    return out


def sine(x0, x1, y, amp, t0=0.0, t1=2 * math.pi, n=80):
    return [(x0 + (x1 - x0) * i / n, y + amp * math.sin(t0 + (t1 - t0) * i / n)) for i in range(n + 1)]


def path(*parts):
    """Concatenate points / point lists, dropping consecutive duplicates."""
    out = []
    for p in parts:
        seq = [p] if isinstance(p, tuple) else p
        for q in seq:
            if not out or math.dist(out[-1], q) > 0.01:
                out.append(q)
    return out


class Glyph:
    def __init__(self, w):
        self.w = w
        self.parts = []
        self.clip = None

    def s(self, pts, w=None, cap="flat", clip=None):
        width = (w or self.w)
        ls = LineString(pts)
        g = ls.buffer(width / 2, cap_style=cap, join_style="mitre", mitre_limit=4.0, quad_segs=24)
        if clip:
            g = g.intersection(box(*clip))
        self.parts.append(g)
        return self

    def round(self, pts, w=None):
        g = LineString(pts).buffer((w or self.w) / 2, cap_style="round", join_style="round", quad_segs=24)
        self.parts.append(g)
        return self

    def dot(self, x, y, r):
        self.parts.append(Point(x, y).buffer(r, quad_segs=32))
        return self

    def ellipse(self, x, y, rx, ry):
        self.parts.append(affinity.scale(Point(x, y).buffer(1, quad_segs=32), rx, ry))
        return self

    def fill(self, pts):
        self.parts.append(Polygon(pts))
        return self

    def add(self, geom):
        self.parts.append(geom)
        return self

    def geometry(self):
        g = unary_union(self.parts) if self.parts else Polygon()
        if self.clip:
            g = g.intersection(box(*self.clip))
        return g
