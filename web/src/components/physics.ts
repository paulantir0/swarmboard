// Rigid-body sailboat on the Gerstner sea, and a crew that rides it.
//
// The hull is a set of sample points along its bottom. Each point below the local water surface
// gets a buoyant force proportional to how deep it is (capped at the hull's height there), plus
// drag against the water particles' own orbital velocity. The boat sails: wind thrust along its
// heading, hull drag through the (drifting) water, and the slope of the sea under it, so it surges
// down wave faces and slows climbing them. The sail's push acts above the centre of mass, so the
// bow dips a little while it drives. At each end of its run it luffs, coasts, and turns: the hull
// foreshortens through bow-on (scale x through 0) and comes out on the other tack, then the thrust
// builds again. Integrated at a fixed 120 Hz with semi-implicit Euler; the renderer interpolates.
//
// Spring constant and centre of mass come from the drawn waterline: on calm water the summed
// buoyancy of the design draft equals the weight, and the centre of mass sits over its centroid,
// so the boat floats level exactly where it was drawn.
//
// Each crew member is a mass on a damped spring along the deck, anchored at its slot. In the boat's
// frame it feels the deck's acceleration (inertia) and gravity along the tilted deck, so it sways
// and leans downhill; hops are ballistic in world space under gravity, landing back on the deck.

import { type Band, type Sample, surfaceAt, pushSpring } from './water'

export const STEP = 1 / 120

export interface HullDef {
  /** Bottom points relative to the pivot (waterline centre), CSS px, y down; bow first, bow right. */
  pts: Float32Array
  /** Column height above each bottom point (max useful depth). */
  cap: Float32Array
  /** Length of the hull (CSS px), for inertia and the wake. */
  length: number
  /** Local x of the bow and stern tips (bow to the right). */
  bowX: number
  sternX: number
  /** Height of the sails' centre of effort above the waterline (CSS px). */
  ce: number
}

export interface Course {
  /** Pivot x range to tack across; too narrow to sail means heave-to at `hold`. */
  xMin: number
  xMax: number
  hold: number
  /** Speed through the water at full drive (px/s). */
  cruise: number
  /** The band's current (px/s, leftwards). */
  current: number
}

const sample: Sample = { y: 0, vx: 0, vy: 0, slope: 0 }
const smooth = (u: number) => u * u * (3 - 2 * u)
/** Seconds to come about, and to build full drive afterwards. */
const TURN_T = 2.6
const RAMP_T = 2.4
/** Hull drag through the water, per second (sets the coasting distance). */
const DRAG = 0.85

export class BoatSim {
  // centre of mass, world (CSS px), and angle (rad, + clockwise)
  x = 0
  y = 0
  a = 0
  vx = 0
  vy = 0
  w = 0
  // previous step, for interpolation
  px = 0
  py = 0
  pa = 0
  psx = 1
  // last accelerations
  ax = 0
  ay = 0
  aa = 0
  /** Centre of mass relative to the pivot (local, heading right). */
  cx = 0
  cy = 0
  K = 1
  I = 1
  cPitch = 0
  draftRef = 1
  g: number
  hull: HullDef
  /** Visual offset: the water surface sits this far above the band's wave line. */
  surf: number
  // sailing
  /** Heading: +1 bow right, -1 bow left. */
  dir = 1
  /** Horizontal scale of the hull as drawn: dir on a tack, through 0 while turning. */
  sx = 1
  turning = false
  turnU = 0
  ramp = 1
  /** Completed turns (Sea.tsx cheers on each). */
  turns = 0
  /** Sail drive this step (px/s^2), for the crew and the wake. */
  drive = 0

  constructor(hull: HullDef, g: number, surf: number) {
    this.hull = hull
    this.g = g
    this.surf = surf
    const { pts } = hull
    const n = pts.length / 2
    let sum = 0
    let mx = 0
    let cnt = 0
    for (let i = 0; i < n; i++) {
      const d = Math.max(0, pts[2 * i + 1])
      sum += d
      mx += d * pts[2 * i]
      if (d > 0) cnt++
    }
    sum = Math.max(sum, 1e-3)
    this.K = g / sum
    this.cx = mx / sum
    this.cy = 0
    this.draftRef = sum / Math.max(1, cnt)
    // A tall rig and a ballast keel give a sailboat a lot of pitch inertia.
    const gyr = hull.length * 0.4
    this.I = gyr * gyr
    // Pitch damping (radiation and keel drag), about 0.7 of critical against the waterplane's
    // restoring moment, so it follows the swell without ringing.
    let kp = 0
    for (let i = 0; i < n; i++) {
      const d = Math.max(0, pts[2 * i + 1])
      if (d > 0) kp += this.K * (pts[2 * i] - this.cx) ** 2
    }
    this.cPitch = 2 * 0.7 * Math.sqrt(Math.max(1, kp) * this.I)
  }

  /** The hull's x scale for physics: foreshortened while turning, never thinner than a third. */
  private sxp(sx = this.sx) {
    const m = Math.max(0.35, Math.abs(sx))
    return (sx < 0 || (sx === 0 && this.dir < 0) ? -m : m)
  }

  /** Pivot (waterline centre) in world space for a COM pose. */
  pivot(x: number, y: number, a: number, sx = this.sx): [number, number] {
    const c = Math.cos(a)
    const s = Math.sin(a)
    const lx = this.cx * this.sxp(sx)
    return [x - (lx * c - this.cy * s), y - (lx * s + this.cy * c)]
  }

  /** Place the boat at rest on the water at pivot x, heading `dir`. */
  place(band: Band, px: number, t: number, dir: number) {
    this.dir = dir
    this.sx = dir
    this.psx = dir
    this.turning = false
    this.ramp = 0
    surfaceAt(band, px, t, sample)
    const a = Math.atan(sample.slope) * 0.6
    const c = Math.cos(a)
    const s = Math.sin(a)
    const lx = this.cx * dir
    this.x = px + lx * c - this.cy * s
    this.y = sample.y - this.surf + lx * s + this.cy * c
    this.a = a
    this.vx = this.vy = this.w = 0
    this.px = this.x
    this.py = this.y
    this.pa = this.a
  }

  step(band: Band, t: number, h: number, course: Course) {
    const { pts, cap } = this.hull
    const n = pts.length / 2
    const c = Math.cos(this.a)
    const s = Math.sin(this.a)
    const sp = this.sxp()
    const pivotX = this.pivot(this.x, this.y, this.a)[0]
    // The text's drift is mostly surface (Stokes) drift; the boat feels about half of it.
    const wvx = -course.current * 0.5
    let fx = 0
    // Foreshortened mid-turn, the hull's restoring moment shrinks with sp^2; so does what we see of
    // its inertia and damping, which keeps the motion the same speed all the way round.
    const s2 = sp * sp
    let fy = this.g - 0.25 * this.vy
    let tq = -this.cPitch * s2 * this.w
    const cd = 2.6 / n
    let slope = 0
    let wet = 0
    for (let i = 0; i < n; i++) {
      const lx = pts[2 * i] * sp - this.cx * sp
      const ly = pts[2 * i + 1] - this.cy
      const rx = lx * c - ly * s
      const ry = lx * s + ly * c
      surfaceAt(band, this.x + rx, t, sample)
      let d = this.y + ry - (sample.y - this.surf)
      if (d <= 0) continue
      d = Math.min(d, cap[i])
      const frac = Math.min(1.5, d / this.draftRef)
      const pvx = this.vx - this.w * ry
      const pvy = this.vy + this.w * rx
      const ex = -cd * frac * (pvx - sample.vx - wvx) * 0.12
      const ey = -this.K * d - cd * frac * (pvy - sample.vy)
      fx += ex
      fy += ey
      tq += rx * ey - ry * ex * s2
      slope += sample.slope * frac
      wet += frac
    }
    // Hull drag through the water (which drifts with the current) and the sea's slope: a boat on
    // a face that falls away ahead of it slides down it.
    fx += -DRAG * (this.vx - wvx)
    if (wet > 0) fx += this.g * 0.42 * Math.max(-0.4, Math.min(0.4, slope / wet))

    // Sailing: drive on a tack; luff, coast and come about at the ends of the run.
    let drive = 0
    if (course.xMax - course.xMin > 24) {
      if (!this.turning) {
        const coast = Math.max(0, this.vx * this.dir) / DRAG
        if ((this.dir > 0 && pivotX > course.xMax - coast * 0.85) || (this.dir < 0 && pivotX < course.xMin + coast * 0.85)) {
          this.turning = true
          this.turnU = 0
        }
      }
      if (this.turning) {
        this.turnU = Math.min(1, this.turnU + h / TURN_T)
        this.sx = this.dir * Math.cos(Math.PI * smooth(this.turnU))
        if (this.turnU >= 1) {
          this.turning = false
          this.dir = -this.dir
          this.sx = this.dir
          this.ramp = 0
          this.turns++
        }
      } else {
        this.ramp = Math.min(1, this.ramp + h / RAMP_T)
        drive = DRAG * course.cruise * smooth(this.ramp) * this.dir
      }
      // Soft walls, in case a big set carries it past the ends.
      if (pivotX > course.xMax + 12) fx -= 3 * (pivotX - course.xMax - 12)
      if (pivotX < course.xMin - 12) fx -= 3 * (pivotX - course.xMin + 12)
    } else {
      fx += -0.25 * (pivotX - course.hold)
    }
    fx += drive
    // The sails push above the centre of mass, so driving trims the bow down a touch (the keel's
    // lateral resistance takes most of that moment; a third of it reaches pitch).
    tq += drive * this.hull.ce * 0.3
    this.drive = drive

    this.ax = fx
    this.ay = fy
    this.aa = tq / (this.I * s2)
    this.px = this.x
    this.py = this.y
    this.pa = this.a
    this.psx = this.sx
    this.vx += this.ax * h
    this.vy += this.ay * h
    this.w += this.aa * h
    this.x += this.vx * h
    this.y += this.vy * h
    this.a += this.w * h
  }

  /**
   * The hull pushes the water: nodes under the hull follow its vertical motion, a bow wave rises
   * ahead of the stem and the stern leaves a trough, both growing with speed through the water.
   */
  disturb(band: Band, weight: number, current: number) {
    if (weight <= 0) return
    const { pts, bowX, sternX, length } = this.hull
    const n = pts.length / 2
    const c = Math.cos(this.a)
    const s = Math.sin(this.a)
    const sp = this.sxp()
    for (let i = 0; i < n; i += 2) {
      const lx = (pts[2 * i] - this.cx) * sp
      const ly = pts[2 * i + 1] - this.cy
      const rx = lx * c - ly * s
      const vy = this.vy + this.w * rx
      pushSpring(band, this.x + rx, length * 0.06, vy * 2 * weight)
    }
    const ox = this.pivot(this.x, this.y, this.a)[0]
    const bow = ox + bowX * sp * c
    const stern = ox + sternX * sp * c
    const ahead = Math.sign(bowX * sp - sternX * sp) || 1
    const speed = Math.min(60, Math.max(0, (this.vx + current) * this.dir))
    const k = Math.abs(this.sx) * weight
    pushSpring(band, bow + ahead * length * 0.02, length * 0.035, -speed * 4.5 * k)
    pushSpring(band, bow - ahead * length * 0.08, length * 0.05, speed * 2 * k)
    pushSpring(band, stern - ahead * length * 0.02, length * 0.05, speed * 2 * k)
  }
}

export interface Deck {
  /** Deck height (local y) at local x. */
  y: (x: number) => number
}

export class CrewSim {
  slot: number
  s = 0
  sv = 0
  air = false
  hx = 0
  hy = 0
  hvx = 0
  hvy = 0
  /** Feet in world space, now and one step ago (for interpolation). */
  fx = 0
  fy = 0
  pfx = 0
  pfy = 0
  /** Sim time of the last landing (for the squash). */
  landed = -1
  landSpeed = 0
  maxSway: number

  constructor(slot: number, maxSway: number) {
    this.slot = slot
    this.maxSway = maxSway
  }

  hop(boat: BoatSim, deck: Deck, v0: number) {
    if (this.air) return
    const lx = this.slot + this.s
    const ly = deck.y(lx)
    const c = Math.cos(boat.a)
    const s = Math.sin(boat.a)
    const [ox, oy] = boat.pivot(boat.x, boat.y, boat.a)
    const rx = ox + lx * c - ly * s - boat.x
    const ry = oy + lx * s + ly * c - boat.y
    this.air = true
    this.hx = boat.x + rx
    this.hy = boat.y + ry
    this.hvx = boat.vx - boat.w * ry + this.sv * c
    this.hvy = boat.vy + boat.w * rx + this.sv * s - v0
  }

  step(boat: BoatSim, deck: Deck, g: number, h: number, t: number) {
    this.pfx = this.fx
    this.pfy = this.fy
    const c = Math.cos(boat.a)
    const s = Math.sin(boat.a)
    const [ox, oy] = boat.pivot(boat.x, boat.y, boat.a)
    if (this.air) {
      this.hvy += g * h
      this.hx += this.hvx * h
      this.hy += this.hvy * h
      // Back in the boat's frame: below the deck line means we've landed.
      const dx = this.hx - ox
      const dy = this.hy - oy
      const lx = dx * c + dy * s
      const ly = -dx * s + dy * c
      if (this.hvy > 0 && ly >= deck.y(lx)) {
        this.air = false
        this.s = Math.max(-this.maxSway, Math.min(this.maxSway, lx - this.slot))
        // Relative velocity along the deck survives the landing, damped.
        const bvx = boat.vx - boat.w * (this.hy - boat.y)
        const bvy = boat.vy + boat.w * (this.hx - boat.x)
        this.sv = ((this.hvx - bvx) * c + (this.hvy - bvy) * s) * 0.35
        this.landSpeed = Math.abs(-(this.hvx - bvx) * s + (this.hvy - bvy) * c)
        this.landed = t
      } else {
        this.fx = this.hx
        this.fy = this.hy
        return
      }
    } else {
      // Deck tangent at the feet (deck slope plus the boat's angle).
      const lx = this.slot + this.s
      const dl = (deck.y(lx + 2) - deck.y(lx - 2)) / 4
      const ang = boat.a + Math.atan(dl)
      const tx = Math.cos(ang)
      const ty = Math.sin(ang)
      // Acceleration of the deck point: a + alpha x r - w^2 r.
      const ly = deck.y(lx)
      const rx = ox + lx * c - ly * s - boat.x
      const ry = oy + lx * s + ly * c - boat.y
      const apx = boat.ax - boat.aa * ry - boat.w * boat.w * rx
      const apy = boat.ay + boat.aa * rx - boat.w * boat.w * ry
      // In the boat's frame: the pseudo-force of the deck's acceleration, plus gravity along the
      // tilted deck (a floating boat's own acceleration is near zero at rest).
      const f = -(apx * tx + apy * ty) + g * ty
      const k = 7
      const acc = f - k * this.s - 3.2 * this.sv
      this.sv += acc * h
      this.s += this.sv * h
      if (this.s > this.maxSway) {
        this.s = this.maxSway
        this.sv = Math.min(0, this.sv)
      } else if (this.s < -this.maxSway) {
        this.s = -this.maxSway
        this.sv = Math.max(0, this.sv)
      }
    }
    const lx = this.slot + this.s
    const ly = deck.y(lx)
    this.fx = ox + lx * c - ly * s
    this.fy = oy + lx * s + ly * c
  }
}
