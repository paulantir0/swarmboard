"""Swarm Mono glyph definitions.

Every glyph is drawn as centerline strokes on a 600 x 1000 grid and expanded to
outlines at the weight's stroke width. Coordinates use y-up, baseline = 0.
"""
import math

from shapely import affinity

from geom import Glyph, bez, ell, path, se, sine

ADV = 600
XH, CAP, ASC, DESC = 520, 700, 750, -220
LINE_TOP, LINE_BOT = 930, -270  # hhea/typo extents; block elements fill this
OV = 10  # overshoot for round forms
AXIS = 330  # math / dash axis
M = ADV / 2


class Ctx:
    def __init__(self, w):
        self.w = w
        self.h = w / 2
        self.lx, self.rx = 85, 515          # lowercase side edges
        self.Lx, self.Rx = 72, 528          # capital side edges
        self.xL, self.xR = self.lx + self.h, self.rx - self.h
        self.XL, self.XR = self.Lx + self.h, self.Rx - self.h
        self.X = XH - self.h                # x-height centerline
        self.C = CAP - self.h               # cap centerline
        self.A = ASC - self.h               # ascender centerline
        self.B = self.h                     # baseline centerline
        self.D = DESC + self.h              # descender centerline
        self.dot_r = w * 0.64

    def g(self, wscale=1.0):
        return Glyph(self.w * wscale)


GLYPHS = {}


def glyph(*chars):
    def deco(fn):
        for ch in chars:
            GLYPHS[ch] = fn
        return fn
    return deco


def rot180(geom, cx=M, cy=None):
    return affinity.rotate(geom, 180, origin=(cx, cy if cy is not None else CAP / 2))


def mirror_x(geom):
    return affinity.scale(geom, -1, 1, origin=(M, 0))


# ---------------------------------------------------------------- helpers

def o_bowl(c, x0, x1, y0=0, y1=XH, full=True):
    """Centerline superellipse for a bowl whose outer edges span x0..x1, y0..y1."""
    cx, cy = (x0 + x1) / 2, (y0 + y1) / 2
    rx = (x1 - x0) / 2 - c.h
    ry = (y1 - y0) / 2 - c.h + OV
    return cx, cy, rx, ry


def arch(c, x0, x1, top=None):
    """n-style shoulder: from left stem centerline x0 to right stem x1, legs to baseline."""
    top = XH + OV - c.h if top is None else top
    r = (x1 - x0) / 2
    ry = min(XH * 0.56, r * 1.25)
    cy = top - ry
    return path(se((x0 + x1) / 2, cy, r, ry, 180, 0), (x1, 0))


def cut_c(c, cx, cy, rx, ry, a0, a1, xmax):
    """Open bowl (c, C, G...) with vertically cut terminals at xmax."""
    g = c.g()
    g.s(se(cx, cy, rx, ry, a0, a1), clip=(-50, -400, xmax, 1200))
    return g


# ---------------------------------------------------------------- lowercase

@glyph("a")
def _a(c):
    g = c.g()
    r = 95
    g.s(path((c.lx + 40, c.X), (c.xR - r, c.X), se(c.xR - r, c.X - r, r, r, 90, 0, n=2.2), (c.xR, 0)))
    yb = 300
    rb = 120
    by = c.B - OV + 4
    g.s(path((c.xR, yb), (c.xL + rb, yb), se(c.xL + rb, (yb + by) / 2, rb, (yb - by) / 2, 90, 270),
             (c.xR - 130, by), bez((c.xR - 130, by), (c.xR - 60, by), (c.xR, by + 30), (c.xR, by + 110))))
    return g


@glyph("b")
def _b(c):
    g = c.g()
    g.s([(c.xL, 0), (c.xL, ASC)])
    cx, cy, rx, ry = o_bowl(c, c.lx, c.rx)
    g.s(se(cx, cy, rx, ry, 0, 360))
    g.clip = (0, 0, ADV, ASC)
    return g


@glyph("c")
def _c(c):
    cx, cy, rx, ry = o_bowl(c, c.lx + 5, c.rx + 5)
    return cut_c(c, cx, cy, rx, ry, 25, 335, c.rx - 10)


@glyph("d")
def _d(c):
    g = c.g()
    g.s([(c.xR, 0), (c.xR, ASC)])
    cx, cy, rx, ry = o_bowl(c, c.lx, c.rx)
    g.s(se(cx, cy, rx, ry, 0, 360))
    g.clip = (0, 0, ADV, ASC)
    return g


@glyph("e")
def _e(c):
    g = c.g()
    cx, cy, rx, ry = o_bowl(c, c.lx, c.rx)
    yb = cy + 15
    g.s(path((c.xL, yb), (cx + rx, yb)))
    g.s(se(cx, cy, rx, ry, 8, 330), clip=(0, -100, c.rx - 8, 900))
    return g


@glyph("f")
def _f(c):
    g = c.g()
    fx = M - 50
    g.s(path((fx, 0), (fx, c.A - 150), bez((fx, c.A - 150), (fx, c.A + OV), (fx + 60, c.A + OV), (c.xR + 10, c.A - 20))))
    g.s([(c.lx + 25, c.X), (c.rx - 40, c.X)])
    g.clip = (0, 0, c.rx, ASC + OV)
    return g


@glyph("g")
def _g(c):
    g = c.g()
    cx, cy, rx, ry = o_bowl(c, c.lx, c.rx)
    g.s(se(cx, cy, rx, ry, 0, 360))
    y1 = c.D + 130
    g.s(path((c.xR, XH), (c.xR, y1), bez((c.xR, y1), (c.xR, c.D - OV + 10), (c.xR - 80, c.D - OV), (c.xL + 20, c.D + 10))))
    g.clip = (0, DESC - OV, ADV, XH)
    return g


@glyph("h")
def _h(c):
    g = c.g()
    g.s([(c.xL, 0), (c.xL, ASC)])
    g.s(arch(c, c.xL, c.xR))
    return g


@glyph("i")
def _i(c):
    g = c.g()
    sx = M + 10
    g.s([(c.lx + 30, c.X), (sx, c.X), (sx, 0)])
    g.s([(c.lx + 10, c.B), (c.rx - 10, c.B)])
    g.dot(sx - 5, XH + 175, c.dot_r)
    return g


@glyph("j")
def _j(c):
    g = c.g()
    jx = M + 70
    g.s(path((c.lx + 50, c.X), (jx, c.X), (jx, 0), bez((jx, 0), (jx, c.D - 10), (jx - 60, c.D), (c.lx + 30, c.D))))
    g.dot(jx - 5, XH + 175, c.dot_r)
    return g


@glyph("k")
def _k(c):
    g = c.g()
    g.s([(c.xL, 0), (c.xL, ASC)])
    g.s([(c.xR + 25, XH + 25), (c.xL, 165)], clip=(0, 0, ADV, XH))
    g.s([(c.xL + 120, 280), (c.xR + 25, -25)], clip=(0, 0, ADV, XH))
    g.clip = (0, 0, c.rx + 5, ASC)
    return g


@glyph("l")
def _l(c):
    g = c.g()
    sx = M - 10
    g.s(path((c.lx + 20, c.A), (sx, c.A), (sx, 170), bez((sx, 170), (sx, c.B), (sx + 50, c.B), (c.rx - 5, c.B))))
    return g


@glyph("m")
def _m(c):
    g = c.g(0.86)
    h = g.w / 2
    x0, x2 = 50 + h, 550 - h
    x1 = M
    g.s([(x0, 0), (x0, XH)])
    top = XH + OV - h
    for a, b in ((x0, x1), (x1, x2)):
        r = (b - a) / 2
        ry = 260
        g.s(path(se((a + b) / 2, top - ry, r, ry, 180, 0), (b, 0)))
    return g


@glyph("n")
def _n(c):
    g = c.g()
    g.s([(c.xL, 0), (c.xL, XH)])
    g.s(arch(c, c.xL, c.xR))
    return g


@glyph("o")
def _o(c):
    g = c.g()
    g.s(se(*o_bowl(c, c.lx, c.rx), 0, 360))
    return g


@glyph("p")
def _p(c):
    g = c.g()
    g.s([(c.xL, DESC), (c.xL, XH)])
    g.s(se(*o_bowl(c, c.lx, c.rx), 0, 360))
    g.clip = (0, DESC, ADV, XH + OV)
    return g


@glyph("q")
def _q(c):
    g = c.g()
    g.s([(c.xR, DESC), (c.xR, XH)])
    g.s(se(*o_bowl(c, c.lx, c.rx), 0, 360))
    g.clip = (0, DESC, ADV, XH + OV)
    return g


@glyph("r")
def _r(c):
    g = c.g()
    sx = c.xL + 70
    g.s([(c.lx + 10, c.X), (sx, c.X), (sx, 0)])
    g.s(bez((sx, c.X - 170), (sx + 40, c.X + OV), (c.xR - 90, c.X + OV), (c.xR + 15, c.X - 50)))
    g.s([(c.lx + 10, c.B), (c.rx - 60, c.B)])
    g.clip = (0, 0, c.rx, XH + OV)
    return g


def s_shape(c, top, bot, x0, x1, upper_shrink=12):
    """S spine between centerline top/bot, outer edges x0..x1."""
    h = c.h
    mid = (top + bot) / 2 + 8
    rxu = (x1 - x0) / 2 - h - upper_shrink
    rxl = (x1 - x0) / 2 - h
    cx = (x0 + x1) / 2
    up = se(cx, (top + mid) / 2, rxu, (top - mid) / 2, 25, 270, n=2.25)
    lo = se(cx, (mid + bot) / 2, rxl, (mid - bot) / 2, 90, -155, n=2.25)
    return path(up, lo)


@glyph("s")
def _s(c):
    g = c.g()
    g.s(s_shape(c, c.X + OV, c.B - OV, c.lx + 5, c.rx - 5))
    return g


@glyph("t")
def _t(c):
    g = c.g()
    tx = M - 50
    g.s(path((tx, 660), (tx, 160), bez((tx, 160), (tx, c.B - OV), (tx + 60, c.B - OV), (c.rx - 10, c.B + 10))))
    g.s([(c.lx + 15, c.X), (c.rx - 40, c.X)])
    return g


@glyph("u")
def _u(c):
    g = c.g()
    g.s([(c.xR, 0), (c.xR, XH)])
    r = (c.xR - c.xL) / 2
    ry = min(XH * 0.56, r * 1.25)
    bot = c.B - OV
    g.s(path((c.xL, XH), se(M, bot + ry, r, ry, 180, 360)))
    return g


@glyph("v")
def _v(c):
    g = c.g()
    g.s([(c.xL - 15, XH + 40), (M, 0), (c.xR + 15, XH + 40)])
    g.clip = (0, 0, ADV, XH)
    return g


@glyph("w")
def _w(c):
    g = c.g(0.86)
    h = g.w / 2
    g.s([(40 + h, XH + 40), (165, 0), (M, XH - 150), (435, 0), (560 - h, XH + 40)])
    g.clip = (0, 0, ADV, XH)
    return g


@glyph("x")
def _x(c):
    g = c.g()
    g.s([(c.xL - 10, XH + 40), (c.xR + 10, -40)])
    g.s([(c.xR + 10, XH + 40), (c.xL - 10, -40)])
    g.clip = (c.lx - 10, 0, c.rx + 10, XH)
    return g


@glyph("y")
def _y(c):
    g = c.g()
    g.s([(c.xL - 15, XH + 40), (M + 5, 30)])
    # right arm runs straight through to the descender
    p0, p1 = (c.xR + 15, XH + 40), (M + 5, 30)
    t = (DESC - 40 - p0[1]) / (p1[1] - p0[1])
    g.s([p0, (p0[0] + (p1[0] - p0[0]) * t, DESC - 40)])
    g.clip = (0, DESC, ADV, XH)
    return g


@glyph("z")
def _z(c):
    g = c.g()
    g.s([(c.lx + 10, c.X), (c.xR, c.X), (c.xL, c.B), (c.rx, c.B)])
    g.clip = (c.lx, 0, c.rx, XH)
    return g


# ---------------------------------------------------------------- uppercase

def cap_bowl(c, x0, ytop, ybot, xright):
    """D/B/P style bowl: horizontal from stem, round right side, back to stem."""
    r = (ytop - ybot) / 2
    rx = min(r, xright - x0 - 40)
    return path((x0, ytop), (xright - rx, ytop), se(xright - rx, (ytop + ybot) / 2, rx, r, 90, -90, n=2.4), (x0, ybot))


@glyph("A")
def _A(c):
    g = c.g()
    g.s([(c.XL - 25, -40), (M, CAP + 60), (c.XR + 25, -40)])
    g.s([(c.XL + 40, 230), (c.XR - 40, 230)])
    g.clip = (c.Lx - 20, 0, c.Rx + 20, CAP)
    return g


@glyph("B")
def _B(c):
    g = c.g()
    ym = 385
    g.s([(c.XL, 0), (c.XL, CAP)])
    g.s(cap_bowl(c, c.XL, c.C, ym, c.XR - 30))
    g.s(cap_bowl(c, c.XL, ym, c.B, c.XR))
    return g


@glyph("C")
def _C(c):
    cx, cy, rx, ry = o_bowl(c, c.Lx + 5, c.Rx + 5, 0, CAP)
    return cut_c(c, cx, cy, rx, ry, 25, 335, c.Rx - 15)


@glyph("D")
def _D(c):
    g = c.g()
    g.s([(c.XL, 0), (c.XL, CAP)])
    r = 210
    g.s(path((c.XL, c.C), (c.XR - r, c.C), se(c.XR - r, CAP / 2, r, c.C - CAP / 2, 90, -90, n=2.5), (c.XL, c.B)))
    return g


@glyph("E")
def _E(c):
    g = c.g()
    g.s([(c.Rx - 15, c.C), (c.XL, c.C), (c.XL, c.B), (c.Rx - 5, c.B)])
    g.s([(c.XL, 370), (c.Rx - 50, 370)])
    return g


@glyph("F")
def _F(c):
    g = c.g()
    g.s([(c.Rx - 5, c.C), (c.XL, c.C), (c.XL, 0)])
    g.s([(c.XL, 360), (c.Rx - 50, 360)])
    return g


@glyph("G")
def _G(c):
    cx, cy, rx, ry = o_bowl(c, c.Lx, c.Rx, 0, CAP)
    g = cut_c(c, cx, cy, rx, ry, 30, 360, c.Rx)
    g.s([(cx + rx, cy), (cx + rx, 160)])
    g.s([(M + 20, 330), (c.Rx, 330)])
    return g


@glyph("H")
def _H(c):
    g = c.g()
    g.s([(c.XL, 0), (c.XL, CAP)])
    g.s([(c.XR, 0), (c.XR, CAP)])
    g.s([(c.XL, 365), (c.XR, 365)])
    return g


@glyph("I")
def _I(c):
    g = c.g()
    g.s([(M, 0), (M, CAP)])
    g.s([(c.Lx + 55, c.C), (c.Rx - 55, c.C)])
    g.s([(c.Lx + 55, c.B), (c.Rx - 55, c.B)])
    return g


@glyph("J")
def _J(c):
    g = c.g()
    r = (c.XR - c.XL) / 2 - 10
    cy = 230
    g.s(path((c.Lx + 90, c.C), (c.XR, c.C), (c.XR, cy), se(c.XR - r, cy, r, cy - c.B + OV, 0, -165)))
    return g


@glyph("K")
def _K(c):
    g = c.g()
    g.s([(c.XL, 0), (c.XL, CAP)])
    g.s([(c.XR + 30, CAP + 40), (c.XL, 210)])
    g.s([(c.XL + 140, 380), (c.XR + 30, -40)])
    g.clip = (0, 0, c.Rx + 5, CAP)
    return g


@glyph("L")
def _L(c):
    g = c.g()
    g.s([(c.XL, CAP), (c.XL, c.B), (c.Rx - 5, c.B)])
    return g


@glyph("M")
def _M(c):
    g = c.g(0.88)
    h = g.w / 2
    x0, x1 = 58 + h, 542 - h
    g.s([(x0, 0), (x0, CAP + 30), (M, 210), (x1, CAP + 30), (x1, 0)])
    g.clip = (0, 0, ADV, CAP)
    return g


@glyph("N")
def _N(c):
    g = c.g()
    g.s([(c.XL, 0), (c.XL, CAP + 20), (c.XR, -20), (c.XR, CAP)])
    g.clip = (0, 0, ADV, CAP)
    return g


@glyph("O")
def _O(c):
    g = c.g()
    g.s(se(*o_bowl(c, c.Lx, c.Rx, 0, CAP), 0, 360))
    return g


@glyph("P")
def _P(c):
    g = c.g()
    g.s([(c.XL, 0), (c.XL, CAP)])
    g.s(cap_bowl(c, c.XL, c.C, 320, c.XR))
    return g


@glyph("Q")
def _Q(c):
    g = c.g()
    g.s(se(*o_bowl(c, c.Lx, c.Rx, 0, CAP), 0, 360))
    g.s([(M + 40, 150), (c.Rx + 10, -150)])
    g.clip = (0, -140, ADV, CAP + 20)
    return g


@glyph("R")
def _R(c):
    g = c.g()
    g.s([(c.XL, 0), (c.XL, CAP)])
    g.s(cap_bowl(c, c.XL, c.C, 330, c.XR))
    g.s([(M - 10, 330), (c.XR + 30, -40)])
    g.clip = (0, 0, c.Rx + 5, CAP)
    return g


@glyph("S")
def _S(c):
    g = c.g()
    g.s(s_shape(c, c.C + OV, c.B - OV, c.Lx + 10, c.Rx - 10, upper_shrink=14))
    return g


@glyph("T")
def _T(c):
    g = c.g()
    g.s([(c.Lx, c.C), (c.Rx, c.C)])
    g.s([(M, 0), (M, CAP)])
    return g


@glyph("U")
def _U(c):
    g = c.g()
    r = (c.XR - c.XL) / 2
    cy = 250
    g.s(path((c.XL, CAP), (c.XL, cy), se(M, cy, r, cy - c.B + OV, 180, 360), (c.XR, CAP)))
    return g


@glyph("V")
def _V(c):
    g = c.g()
    g.s([(c.XL - 25, CAP + 50), (M, 0), (c.XR + 25, CAP + 50)])
    g.clip = (0, 0, ADV, CAP)
    return g


@glyph("W")
def _W(c):
    g = c.g(0.88)
    h = g.w / 2
    g.s([(45 + h, CAP + 40), (160, 0), (M, CAP - 230), (440, 0), (555 - h, CAP + 40)])
    g.clip = (0, 0, ADV, CAP)
    return g


@glyph("X")
def _X(c):
    g = c.g()
    g.s([(c.XL - 10, CAP + 40), (c.XR + 10, -40)])
    g.s([(c.XR + 10, CAP + 40), (c.XL - 10, -40)])
    g.clip = (c.Lx - 10, 0, c.Rx + 10, CAP)
    return g


@glyph("Y")
def _Y(c):
    g = c.g()
    g.s([(c.XL - 20, CAP + 45), (M, 320), (c.XR + 20, CAP + 45)])
    g.s([(M, 330), (M, 0)])
    g.clip = (0, 0, ADV, CAP)
    return g


@glyph("Z")
def _Z(c):
    g = c.g()
    g.s([(c.Lx + 15, c.C), (c.XR, c.C), (c.XL, c.B), (c.Rx, c.B)])
    g.clip = (c.Lx, 0, c.Rx, CAP)
    return g


# ---------------------------------------------------------------- figures

@glyph("0")
def _0(c):
    g = c.g()
    g.s(se(*o_bowl(c, c.lx, c.rx, 0, CAP), 0, 360))
    g.ellipse(M, CAP / 2, c.w * 0.55, c.w * 0.95)
    return g


@glyph("1")
def _1(c):
    g = c.g()
    sx = M + 15
    g.s([(c.lx + 35, c.C - 120), (sx, CAP + 20), (sx, 0)], clip=(0, 0, ADV, CAP))
    g.s([(c.lx + 25, c.B), (c.rx - 15, c.B)])
    g.clip = (0, 0, ADV, CAP)
    return g


@glyph("2")
def _2(c):
    g = c.g()
    ry = 175
    rx = (c.xR - c.xL) / 2
    cy = c.C + OV - ry
    arc = se(M, cy, rx, ry, 160, 0)
    end = arc[-1]
    g.s(path(arc, bez(end, (end[0], cy - 150), (c.xL + 40, 190), (c.xL, c.B)), (c.rx, c.B)))
    g.clip = (c.lx, 0, c.rx, CAP + OV)
    return g


@glyph("3")
def _3(c):
    g = c.g()
    ym = 395
    top, bot = c.C + OV, c.B - OV
    rxu = (c.xR - c.xL) / 2 - 15
    rxl = (c.xR - c.xL) / 2
    up = se(M - 10, (top + ym) / 2, rxu, (top - ym) / 2, 155, -90)
    lo = se(M - 10, (ym + bot) / 2, rxl, (ym - bot) / 2, 90, -155)
    g.s(path(up, lo))
    g.s([(M - 80, ym), (M - 10, ym)])
    return g


@glyph("4")
def _4(c):
    g = c.g()
    sx = c.xR - 50
    g.s([(sx, 0), (sx, CAP + 60), (c.xL - 20, 220), (c.rx + 20, 220)])
    g.clip = (c.lx - 15, 0, c.rx, CAP)
    return g


@glyph("5")
def _5(c):
    g = c.g()
    cyb, ryb = 235, 235 - c.B + OV
    rxb = (c.xR - c.xL) / 2 + 5
    bowl = se(M, cyb, rxb, ryb, 140, -155)
    g.s(path((c.xR - 10, c.C), (c.xL + 20, c.C), (c.xL + 12, 395), bowl))
    return g


def six(c):
    g = c.g()
    cyb, ryb = 240, 240 - c.B + OV
    rxb = (c.xR - c.xL) / 2
    g.s(se(M, cyb, rxb, ryb, 0, 360))
    g.s(se(M, CAP / 2 + 15, rxb, c.C - CAP / 2 - 15 + OV, 52, 195))
    return g


@glyph("6")
def _6(c):
    return six(c)


@glyph("9")
def _9(c):
    g = c.g()
    g.add(rot180(six(c).geometry()))
    return g


@glyph("7")
def _7(c):
    g = c.g()
    g.s(path((c.lx + 5, c.C), (c.xR, c.C), bez((c.xR, c.C), (c.xR - 120, 470), (M - 60, 250), (M - 60, -20))))
    g.clip = (c.lx, 0, c.rx, CAP)
    return g


@glyph("8")
def _8(c):
    g = c.g()
    ym = 385
    top, bot = c.C + OV, c.B - OV
    g.s(se(M, (top + ym) / 2, (c.xR - c.xL) / 2 - 22, (top - ym) / 2, 0, 360))
    g.s(se(M, (ym + bot) / 2, (c.xR - c.xL) / 2, (ym - bot) / 2, 0, 360))
    return g


# ---------------------------------------------------------------- punctuation

def comma_geom(c, x, y):
    """Comma whose head sits on (x, y) = top of the dot."""
    g = c.g()
    r = c.dot_r * 1.05
    g.dot(x, y - r, r)
    g.s(bez((x + r * 0.55, y - r), (x + r * 0.6, y - r - 80), (x - 20, y - r - 160), (x - 85, y - r - 210)),
        w=c.w * 0.75)
    return g.geometry()


@glyph(" ")
def _space(c):
    return c.g()


@glyph(" ")
def _nbsp(c):
    return c.g()


@glyph("!")
def _excl(c):
    g = c.g()
    g.s([(M, CAP), (M, 220)], w=c.w * 1.05)
    g.dot(M, c.dot_r, c.dot_r)
    return g


@glyph('"')
def _quot(c):
    g = c.g()
    for x in (M - 85, M + 85):
        g.s([(x, CAP + 20), (x, CAP - 200)])
    return g


@glyph("#")
def _hash(c):
    g = c.g(0.9)
    for x in (M - 90, M + 90):
        g.s([(x - 15, 40), (x + 15, 660)])
    for y in (235, 455):
        g.s([(c.lx - 15, y), (c.rx + 15, y)])
    return g


@glyph("$")
def _dollar(c):
    g = c.g()
    g.s(s_shape(c, c.C + OV - 40, c.B - OV + 40, c.Lx + 15, c.Rx - 15))
    g.s([(M, -60), (M, c.B + 60)])
    g.s([(M, c.C - 60), (M, CAP + 60)])
    return g


@glyph("%")
def _pct(c):
    g = c.g(0.85)
    r = 92
    g.s(se(c.lx + r + 10, CAP - r - 25, r, r + 15, 0, 360))
    g.s(se(c.rx - r - 10, r + 25, r, r + 15, 0, 360))
    g.s([(c.rx, CAP + 30), (c.lx, -30)])
    g.clip = (c.lx, 0, c.rx, CAP)
    return g


@glyph("&")
def _amp(c):
    g = c.g()
    p = path(
        (c.rx + 15, -25),
        (M - 85, 445),
        bez((M - 85, 445), (M - 150, 530), (M - 150, c.C + OV), (M - 30, c.C + OV)),
        bez((M - 30, c.C + OV), (M + 80, c.C + OV), (M + 110, 560), (M + 20, 495)),
        bez((M + 20, 495), (c.xL - 10, 400), (c.xL - 10, 300), (c.xL, 200)),
        bez((c.xL, 200), (c.xL + 10, 60), (M - 60, c.B - OV), (M, c.B - OV)),
        bez((M, c.B - OV), (c.xR - 40, c.B - OV), (c.xR, 150), (c.xR, 300)),
    )
    g.s(p)
    g.clip = (c.lx - 10, 0, c.rx, CAP + OV)
    return g


@glyph("'")
def _apos(c):
    g = c.g()
    g.s([(M, CAP + 20), (M, CAP - 200)])
    return g


@glyph("(")
def _lparen(c):
    g = c.g()
    g.s(bez((M + 110, ASC + 30), (M - 110, 560), (M - 110, 30), (M + 110, -180)))
    g.clip = (0, -170, ADV, ASC + 20)
    return g


@glyph(")")
def _rparen(c):
    g = c.g()
    g.add(mirror_x(_lparen(c).geometry()))
    return g


@glyph("*")
def _ast(c):
    g = c.g(0.9)
    cx, cy, r = M, 470, 175
    for k in range(5):
        a = math.radians(90 + 72 * k)
        g.s([(cx, cy), (cx + r * math.cos(a), cy + r * math.sin(a))])
    return g


@glyph("+")
def _plus(c):
    g = c.g()
    g.s([(M - 195, AXIS), (M + 195, AXIS)])
    g.s([(M, AXIS - 195), (M, AXIS + 195)])
    return g


@glyph(",")
def _comma(c):
    g = c.g()
    g.add(comma_geom(c, M + 15, 2 * c.dot_r * 1.05))
    return g


@glyph("-")
def _hyphen(c):
    g = c.g()
    g.s([(c.lx + 40, AXIS), (c.rx - 40, AXIS)])
    return g


@glyph(".")
def _period(c):
    g = c.g()
    g.dot(M, c.dot_r, c.dot_r)
    return g


@glyph("/")
def _slash(c):
    g = c.g()
    g.s([(c.rx, ASC + 30), (c.lx, -150)])
    g.clip = (0, -130, ADV, ASC)
    return g


@glyph(":")
def _colon(c):
    g = c.g()
    g.dot(M, c.dot_r, c.dot_r)
    g.dot(M, XH - c.dot_r, c.dot_r)
    return g


@glyph(";")
def _semi(c):
    g = c.g()
    g.add(comma_geom(c, M + 15, 2 * c.dot_r * 1.05))
    g.dot(M + 15, XH - c.dot_r, c.dot_r)
    return g


@glyph("<")
def _lt(c):
    g = c.g()
    g.s([(c.xR, AXIS + 230), (c.xL - 10, AXIS), (c.xR, AXIS - 230)])
    g.clip = (c.lx, 0, ADV, CAP)
    return g


@glyph(">")
def _gt(c):
    g = c.g()
    g.add(mirror_x(_lt(c).geometry()))
    return g


@glyph("=")
def _eq(c):
    g = c.g()
    for y in (AXIS - 95, AXIS + 95):
        g.s([(c.lx + 5, y), (c.rx - 5, y)])
    return g


@glyph("?")
def _quest(c):
    g = c.g()
    top = c.C + OV
    ry = 135
    rx = (c.xR - c.xL) / 2 - 5
    arc = se(M, top - ry, rx, ry, 165, -55)
    end = arc[-1]
    g.s(path(arc, bez(end, (end[0] - 50, end[1] - 45), (M, end[1] - 50), (M, 320)), (M, 225)))
    g.dot(M, c.dot_r, c.dot_r)
    return g


@glyph("@")
def _at(c):
    g = c.g(0.82)
    h = g.w / 2
    cx, cy = M, 290
    rx, ry = (c.rx - c.lx) / 2 - h + 45, 345
    outer = se(cx, cy, rx, ry, -25, 300)
    g.s(outer)
    ix = M + 70
    g.s(se(M - 30, cy, 100, 130, 0, 360))
    g.s(path((ix, cy + 140), (ix, cy - 60), bez((ix, cy - 60), (ix, cy - 160), (ix + 60, cy - 190), outer[0])))
    g.clip = (c.lx - 50, -100, c.rx + 50, CAP + 20)
    return g


@glyph("[")
def _lbrack(c):
    g = c.g()
    x = M - 60
    g.s([(c.xR - 20, ASC + 30 - c.h), (x, ASC + 30 - c.h), (x, -160 + c.h), (c.xR - 20, -160 + c.h)])
    return g


@glyph("]")
def _rbrack(c):
    g = c.g()
    g.add(mirror_x(_lbrack(c).geometry()))
    return g


@glyph("\\")
def _bslash(c):
    g = c.g()
    g.add(mirror_x(_slash(c).geometry()))
    return g


@glyph("^")
def _caret(c):
    g = c.g()
    g.s([(c.xL, 360), (M, CAP + 40), (c.xR, 360)])
    g.clip = (0, 0, ADV, CAP)
    return g


@glyph("_")
def _under(c):
    g = c.g()
    g.s([(c.lx - 20, -120), (c.rx + 20, -120)])
    return g


@glyph("`")
def _grave(c):
    g = c.g()
    g.s([(M - 70, CAP + 40), (M + 50, 590)])
    return g


@glyph("{")
def _lbrace(c):
    g = c.g()
    top, bot = ASC + 30 - c.h, -160 + c.h
    cy = (top + bot) / 2
    x = M - 40
    upper = path(bez((c.xR - 10, top), (x - 10, top), (x, top - 30), (x, top - 150)), (x, cy + 130),
                 bez((x, cy + 130), (x, cy + 30), (x - 50, cy), (c.xL - 10, cy)))
    g.s(upper)
    g.add(affinity.scale(g.geometry(), 1, -1, origin=(M, cy)))
    return g


@glyph("}")
def _rbrace(c):
    g = c.g()
    g.add(mirror_x(_lbrace(c).geometry()))
    return g


@glyph("|")
def _bar(c):
    g = c.g()
    g.s([(M, -170), (M, ASC + 30)])
    return g


@glyph("~")
def _tilde(c):
    g = c.g()
    g.round(sine(c.xL - 5, c.xR + 5, AXIS + 10, 62, t0=-0.15 * math.pi, t1=2.15 * math.pi))
    return g


# ---------------------------------------------------------------- extras

@glyph("·")
def _middot(c):
    g = c.g()
    g.dot(M, AXIS, c.dot_r)
    return g


@glyph("•")
def _bullet(c):
    g = c.g()
    g.dot(M, AXIS, c.dot_r * 1.8)
    return g


@glyph("°")
def _degree(c):
    g = c.g(0.85)
    g.s(se(M, CAP - 115, 100, 105, 0, 360, n=2.0))
    return g


@glyph("≈")
def _approx(c):
    g = c.g(0.92)
    for y in (AXIS - 100, AXIS + 100):
        g.round(sine(c.xL - 5, c.xR + 5, y, 40, t0=-0.15 * math.pi, t1=2.15 * math.pi))
    return g


@glyph("∼")
def _tildeop(c):
    g = c.g()
    g.round(sine(c.lx + 10, c.rx - 10, AXIS - 20, 40, t0=-0.1 * math.pi, t1=2.1 * math.pi))
    return g


@glyph("˜")
def _smalltilde(c):
    g = c.g(0.9)
    g.round(sine(M - 140, M + 140, 640, 38, t0=-0.1 * math.pi, t1=2.1 * math.pi))
    return g


@glyph("–")
def _endash(c):
    g = c.g()
    g.s([(c.lx - 10, AXIS), (c.rx + 10, AXIS)])
    return g


@glyph("—")
def _emdash(c):
    g = c.g()
    g.s([(0, AXIS), (ADV, AXIS)])
    return g


@glyph("’")
def _rsquo(c):
    g = c.g()
    g.add(comma_geom(c, M + 15, CAP + 20))
    return g


@glyph("‘")
def _lsquo(c):
    g = c.g()
    geom = comma_geom(c, M + 15, CAP + 20)
    minx, miny, maxx, maxy = geom.bounds
    g.add(affinity.rotate(geom, 180, origin=((minx + maxx) / 2, (miny + maxy) / 2)))
    return g


@glyph("”")
def _rdquo(c):
    g = c.g()
    for dx in (-95, 95):
        g.add(comma_geom(c, M + 15 + dx, CAP + 20))
    return g


@glyph("“")
def _ldquo(c):
    g = c.g()
    for dx in (-95, 95):
        geom = comma_geom(c, M + 15 + dx, CAP + 20)
        minx, miny, maxx, maxy = geom.bounds
        g.add(affinity.rotate(geom, 180, origin=((minx + maxx) / 2, (miny + maxy) / 2)))
    return g


@glyph("…")
def _ellipsis(c):
    g = c.g()
    r = c.dot_r * 0.9
    for x in (110, 300, 490):
        g.dot(x, r, r)
    return g


def arrow_right(c):
    g = c.g()
    y = AXIS
    g.s([(45, y), (535, y)])
    g.s([(395, y + 170), (550, y), (395, y - 170)])
    g.clip = (0, 0, 545, ADV)
    return g


@glyph("→")
def _rarr(c):
    return arrow_right(c)


@glyph("←")
def _larr(c):
    g = c.g()
    g.add(mirror_x(arrow_right(c).geometry()))
    return g


@glyph("↑")
def _uarr(c):
    g = c.g()
    g.add(affinity.rotate(arrow_right(c).geometry(), 90, origin=(M, AXIS)))
    return g


@glyph("↓")
def _darr(c):
    g = c.g()
    g.add(affinity.rotate(arrow_right(c).geometry(), -90, origin=(M, AXIS)))
    return g


@glyph("×")
def _times(c):
    g = c.g()
    d = 150
    g.s([(M - d, AXIS - d), (M + d, AXIS + d)])
    g.s([(M - d, AXIS + d), (M + d, AXIS - d)])
    return g


@glyph("✓")
def _check(c):
    g = c.g()
    g.s([(c.xL - 15, 340), (M - 60, 70), (c.xR + 25, 640)])
    g.clip = (c.lx - 20, 0, c.rx + 20, CAP)
    return g


# ---------------------------------------------------------------- blocks

def _cells(c, period, size, keep):
    g = c.g()
    rows = int((LINE_TOP - LINE_BOT) / period)
    cols = int(ADV / period)
    for j in range(rows):
        for i in range(cols):
            if keep(i, j):
                x0, y0 = i * period, LINE_BOT + j * period
                g.fill([(x0, y0), (x0 + size, y0), (x0 + size, y0 + size), (x0, y0 + size)])
    return g


@glyph("░")
def _shade_light(c):
    # 25%: sparse staggered dots
    return _cells(c, 75, 70, lambda i, j: j % 2 == 0 and (i + j // 2) % 2 == 0)


@glyph("▒")
def _shade_med(c):
    return _cells(c, 75, 70, lambda i, j: (i + j) % 2 == 0)


@glyph("▓")
def _shade_dark(c):
    return _cells(c, 75, 75, lambda i, j: not (j % 2 == 0 and (i + j // 2) % 2 == 0))


def _rect(c, y0, y1):
    g = c.g()
    g.fill([(0, y0), (ADV, y0), (ADV, y1), (0, y1)])
    return g


@glyph("▀")
def _upper_half(c):
    return _rect(c, (LINE_TOP + LINE_BOT) / 2, LINE_TOP)


@glyph("▄")
def _lower_half(c):
    return _rect(c, LINE_BOT, (LINE_TOP + LINE_BOT) / 2)


@glyph("█")
def _full(c):
    return _rect(c, LINE_BOT, LINE_TOP)
