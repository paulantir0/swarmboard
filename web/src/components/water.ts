// The water under the landing page, as physics rather than sine sums.
//
// Each band is a Gerstner (trochoidal) sea: a handful of wave trains, each moving the water's
// particles in circles. A particle whose rest position is x0 sits at
//   x = x0 - sum(dir * A * D * sin(theta)),  y = base + depth - sum(A * D * cos(theta))
//   theta = dir * k * x0 - omega * t + phase,   omega = sqrt(g * k)   (deep-water dispersion)
// so crests are sharp and troughs broad, particles advance on the crests and drift back in the
// troughs, and long swells outrun short chop. D = exp(-k * depth) shrinks the orbits with depth.
// Steepness k * A summed over a band stays below 1, so crests never loop.
//
// Bands further back are the same kind of sea seen in perspective: wavelengths and heights scale
// by p, gravity by p too, so the periods match and the far water simply looks smaller and slower.
//
// On top of that, each band carries a 1D damped spring-mass surface for local disturbances (the
// boat's bow wave and wake, a pointer dragged across the water). It is advected with the band's
// current, so a wake streams away behind the boat.

export const TAU = Math.PI * 2
/** Gravity in screen units (px/s^2): an 820px swell has a period of about 9s. */
export const G = 64
/** Wave trains per band. */
export const NW = 4
/** Spring surface node spacing (CSS px) and the off-screen margin, in nodes. */
export const SX = 8
const PAD = 3

// Front-band spectrum at desktop scale: two swells from the right, a weaker cross swell from the
// left (it makes the sets and the irregular crests), and a short chop.
const SPECTRUM: { lambda: number; steep: number; dir: number }[] = [
  { lambda: 820, steep: 0.19, dir: -1 },
  { lambda: 470, steep: 0.13, dir: -1 },
  { lambda: 640, steep: 0.075, dir: 1 },
  { lambda: 205, steep: 0.085, dir: -1 },
]
const SPECTRUM_AMP = SPECTRUM.reduce((s, w) => s + (w.steep * w.lambda) / TAU, 0)

/** How deep (CSS px of depth per CSS px of row offset) the rows of text sit for orbit decay. */
const DEPTH = 0.55

// Spring surface: wave speed, restoring stiffness, damping (per second), edge damping.
const SPRING_C2 = 105 * 105
const SPRING_K = 7
const SPRING_D = 2.1

export interface Band {
  index: number
  base: number
  p: number
  /** Gravity used for this band (G scaled by perspective). */
  g: number
  k: Float32Array
  amp: Float32Array
  omega: Float32Array
  dir: Float32Array
  phase: Float32Array
  /** Sum of k * A (max crest compression). */
  steep: number
  /** Sum of A: the highest a crest can reach above base. */
  ampMax: number
  /** Current, px/s leftwards: the text drifts with it. */
  drift: number
  alpha: number
  rows: Uint8Array[]
  /** Orbit decay per row and wave train: decay[r * NW + j]. */
  decay: Float32Array
  // spring surface (screen space, node i at x = (i - PAD) * SX)
  h: Float32Array
  v: Float32Array
  f: Float32Array
  tmp: Float32Array
  /** Foam intensity per absolute text column (ring). */
  foam: Float32Array
  // per frame, per visible column: trig of each train, and the row-0 particle
  sn: Float32Array
  cs: Float32Array
  X: Float32Array
  Y: Float32Array
  S: Float32Array
  J: Float32Array
  /** Surface height on a regular screen grid (every LUT_DX px), for culling the band behind. */
  lut: Float32Array
}

export const LUT_DX = 8
export const FOAM_RING = 1024

export interface BandOpts {
  index: number
  base: number
  p: number
  /** Wavelength scale for the viewport (1 at desktop). */
  scale: number
  /** Target highest crest for the front band (CSS px). */
  amp: number
  calm: number
  drift: number
  alpha: number
  cols: number
  rows: number
  width: number
  lineH: number
  rand: () => number
}

export function makeBand(o: BandOpts): Band {
  const k = new Float32Array(NW)
  const amp = new Float32Array(NW)
  const omega = new Float32Array(NW)
  const dir = new Float32Array(NW)
  const phase = new Float32Array(NW)
  const g = G * o.p * o.scale
  // Fit the spectrum's height to the layout's amplitude budget: wavelengths stretch with it, and
  // whatever is left over lowers the steepness, never raises it.
  const stretch = Math.min(1.12, Math.max(0.55, o.amp / SPECTRUM_AMP / o.scale))
  const q = Math.min(1, o.amp / (SPECTRUM_AMP * stretch * o.scale))
  let steep = 0
  let ampMax = 0
  for (let j = 0; j < NW; j++) {
    const s = SPECTRUM[j]
    const jitter = 0.88 + o.rand() * 0.24
    const lambda = s.lambda * stretch * o.scale * o.p * jitter
    k[j] = TAU / lambda
    const sj = s.steep * q * o.calm
    amp[j] = sj / k[j]
    omega[j] = Math.sqrt(g * k[j])
    dir[j] = s.dir
    phase[j] = o.rand() * TAU
    steep += sj
    ampMax += amp[j]
  }
  const decay = new Float32Array(o.rows * NW)
  for (let r = 0; r < o.rows; r++) for (let j = 0; j < NW; j++) decay[r * NW + j] = Math.exp(-k[j] * r * o.lineH * DEPTH)
  const nodes = Math.ceil(o.width / SX) + PAD * 2 + 1
  const lutN = Math.ceil(o.width / LUT_DX) + 2
  return {
    index: o.index,
    base: o.base,
    p: o.p,
    g,
    k,
    amp,
    omega,
    dir,
    phase,
    steep,
    ampMax,
    drift: o.drift,
    alpha: o.alpha,
    rows: [],
    decay,
    h: new Float32Array(nodes),
    v: new Float32Array(nodes),
    f: new Float32Array(nodes),
    tmp: new Float32Array(nodes),
    foam: new Float32Array(FOAM_RING),
    sn: new Float32Array(o.cols * NW),
    cs: new Float32Array(o.cols * NW),
    X: new Float32Array(o.cols),
    Y: new Float32Array(o.cols),
    S: new Float32Array(o.cols),
    J: new Float32Array(o.cols),
    lut: new Float32Array(lutN),
  }
}

/** Water state at a screen x: surface y, particle velocity, slope. Reused to avoid allocation. */
export interface Sample {
  y: number
  vx: number
  vy: number
  slope: number
}

/**
 * The Gerstner surface (row 0, no spring) at screen x and time t. The surface is parametric in the
 * rest position, so invert x(x0) by fixed-point iteration (a contraction while steepness < 1).
 */
export function surfaceAt(b: Band, x: number, t: number, out: Sample): Sample {
  const { k, amp, omega, dir, phase } = b
  let x0 = x
  for (let it = 0; it < 4; it++) {
    let s = 0
    for (let j = 0; j < NW; j++) s += dir[j] * amp[j] * Math.sin(dir[j] * k[j] * x0 - omega[j] * t + phase[j])
    x0 = x + s
  }
  let y = b.base
  let vx = 0
  let vy = 0
  let jx = 1
  let jy = 0
  for (let j = 0; j < NW; j++) {
    const th = dir[j] * k[j] * x0 - omega[j] * t + phase[j]
    const c = Math.cos(th)
    const s = Math.sin(th)
    const a = amp[j]
    y -= a * c
    vx += dir[j] * a * omega[j] * c
    vy -= a * omega[j] * s
    jx -= k[j] * a * c
    jy += dir[j] * k[j] * a * s
  }
  out.y = y + springAt(b, x)
  out.vx = vx
  out.vy = vy
  out.slope = jy / Math.max(0.05, jx)
  return out
}

/** Spring-surface displacement at screen x (positive = down, like screen y). */
export function springAt(b: Band, x: number): number {
  const u = x / SX + PAD
  const i = Math.floor(u)
  const h = b.h
  if (i < 0 || i >= h.length - 1) return 0
  const f = u - i
  return h[i] + (h[i + 1] - h[i]) * f
}

export function springSlope(b: Band, x: number): number {
  const u = x / SX + PAD
  const i = Math.floor(u)
  const h = b.h
  if (i < 0 || i >= h.length - 1) return 0
  return (h[i + 1] - h[i]) / SX
}

/** Add a vertical acceleration (px/s^2, + down) to the spring nodes, Gaussian around x. */
export function pushSpring(b: Band, x: number, width: number, accel: number) {
  const f = b.f
  const i0 = Math.max(1, Math.floor((x - width * 2.5) / SX + PAD))
  const i1 = Math.min(f.length - 2, Math.ceil((x + width * 2.5) / SX + PAD))
  const inv = 1 / (width * width)
  for (let i = i0; i <= i1; i++) {
    const d = (i - PAD) * SX - x
    f[i] += accel * Math.exp(-d * d * inv)
  }
}

/** One semi-implicit Euler step of the spring surface, then advect it with the current. */
export function stepSpring(b: Band, dt: number) {
  const { h, v, f, tmp } = b
  const n = h.length
  const c2 = SPRING_C2 / (SX * SX)
  for (let i = 1; i < n - 1; i++) {
    // Extra damping in the off-screen margin so nothing reflects back from the edges.
    const edge = i < PAD + 1 || i > n - PAD - 2 ? 6 : 0
    const a = c2 * (h[i - 1] + h[i + 1] - 2 * h[i]) - SPRING_K * h[i] - (SPRING_D + edge) * v[i] + f[i]
    v[i] += a * dt
    f[i] = 0
  }
  for (let i = 1; i < n - 1; i++) h[i] += v[i] * dt
  h[0] = h[1]
  h[n - 1] = h[n - 2]
  v[0] = v[n - 1] = 0
  f[0] = f[n - 1] = 0
  // Advect leftwards with the current (sample upstream, linear interpolation).
  const s = (b.drift * dt) / SX
  if (s > 0) {
    for (let i = 0; i < n - 1; i++) tmp[i] = h[i] + (h[i + 1] - h[i]) * s
    tmp[n - 1] = h[n - 1]
    h.set(tmp)
    for (let i = 0; i < n - 1; i++) tmp[i] = v[i] + (v[i + 1] - v[i]) * s
    tmp[n - 1] = v[n - 1]
    v.set(tmp)
  }
}

export function resetSpring(b: Band) {
  b.h.fill(0)
  b.v.fill(0)
  b.f.fill(0)
  b.foam.fill(0)
}
