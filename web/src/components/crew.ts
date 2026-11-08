// The crew and, when available, the painted boat: hand-painted sprite sheets described by
// /crew/manifest.json (watercolour Clawds pre-rendered offline; the medium is too slow to paint
// live). If the manifest or a sheet is missing, the page falls back to the pixel Clawds in
// clawd.ts and the SVG boat in Boat.tsx, so it never looks broken.
//
// Both kinds of crew share one animation state machine: an idle loop by default, staggered
// blink and look one-shots, a wave loop (or the accessory's own loop: spyglass peering, flag
// waving) for some of them, a squash when a hop lands, the odd cheer.

import { ARM_LIFT, ARM_OUT, ARM_REST, ARM_UP, EYE_C, EYE_HAPPY, EYE_L, EYE_R, EYE_SHUT, type Kind, type Pose } from './clawd'

export type Anim = 'idle' | 'blink' | 'look' | 'wave' | 'squash' | 'cheer'
export const ANIMS: Anim[] = ['idle', 'blink', 'look', 'wave', 'squash', 'cheer']

export interface AnimDef {
  row: number
  frames: number
  loop: boolean
}

export interface Member {
  id: string
  color?: string
  accessory?: string
  sheet: string
  anims: Record<Anim, AnimDef>
}

export interface Manifest {
  version: number
  fps: number
  crew?: {
    frame: { w: number; h: number }
    display: { height: number }
    anchor: { x: number; y: number }
    members: Member[]
  }
  boat?: {
    sheet: string
    frame: { w: number; h: number }
    frames: number
    display: { width: number }
    waterline: number
    hull: [number, number][]
    deck: [[number, number], [number, number]]
    slots: number[]
  }
}

export interface Assets {
  manifest: Manifest
  crew: HTMLImageElement[] | null
  boat: HTMLImageElement | null
}

const BASE = '/crew/'

function loadImage(src: string): Promise<HTMLImageElement> {
  return new Promise((resolve, reject) => {
    const img = new Image()
    img.decoding = 'async'
    img.onload = () => resolve(img)
    img.onerror = () => reject(new Error(`image ${src}`))
    img.src = src
  })
}

const isNum = (v: unknown): v is number => typeof v === 'number' && Number.isFinite(v)

function validCrew(c: Manifest['crew']): boolean {
  if (!c || !Array.isArray(c.members) || c.members.length === 0) return false
  if (!isNum(c.frame?.w) || !isNum(c.frame?.h) || !isNum(c.display?.height) || !isNum(c.anchor?.x) || !isNum(c.anchor?.y)) return false
  return c.members.every((m) => typeof m.sheet === 'string' && m.anims && ANIMS.every((a) => !m.anims[a] || (isNum(m.anims[a].row) && isNum(m.anims[a].frames))) && m.anims.idle)
}

function validBoat(b: Manifest['boat']): boolean {
  if (!b || typeof b.sheet !== 'string') return false
  if (!isNum(b.frame?.w) || !isNum(b.frame?.h) || !isNum(b.frames) || !isNum(b.display?.width) || !isNum(b.waterline)) return false
  if (!Array.isArray(b.hull) || b.hull.length < 3 || !Array.isArray(b.deck) || b.deck.length !== 2) return false
  return Array.isArray(b.slots) && b.slots.length >= 1
}

/** Manifest plus decoded sheets, or null for the fallback. Never throws. */
export async function loadAssets(): Promise<Assets | null> {
  try {
    const res = await fetch(`${BASE}manifest.json`, { cache: 'no-cache' })
    if (!res.ok || !(res.headers.get('content-type') ?? '').includes('json')) return null
    const manifest = (await res.json()) as Manifest
    if (!manifest || !isNum(manifest.fps) || manifest.fps <= 0) return null
    const crewOk = validCrew(manifest.crew)
    const boatOk = validBoat(manifest.boat)
    const [crew, boat] = await Promise.all([
      crewOk ? Promise.all(manifest.crew!.members.map((m) => loadImage(BASE + m.sheet))).catch(() => null) : Promise.resolve(null),
      boatOk ? loadImage(BASE + manifest.boat!.sheet).catch(() => null) : Promise.resolve(null),
    ])
    if (!crew && !boat) return null
    return { manifest, crew, boat }
  } catch {
    return null
  }
}

/**
 * Downscale a sheet to an exact device-pixel size with good filtering (halving steps, then one
 * high-quality pass), so per-frame draws are 1:1 and crisp.
 */
export function downscale(img: CanvasImageSource, sw: number, sh: number, dw: number, dh: number): HTMLCanvasElement {
  let src: CanvasImageSource = img
  let w = sw
  let h = sh
  while (w / 2 >= dw * 1.05 && h / 2 >= dh * 1.05) {
    const c = document.createElement('canvas')
    c.width = Math.max(1, Math.round(w / 2))
    c.height = Math.max(1, Math.round(h / 2))
    const g = c.getContext('2d')
    if (!g) break
    g.imageSmoothingQuality = 'high'
    g.drawImage(src, 0, 0, w, h, 0, 0, c.width, c.height)
    src = c
    w = c.width
    h = c.height
  }
  const out = document.createElement('canvas')
  out.width = Math.max(1, dw)
  out.height = Math.max(1, dh)
  const g = out.getContext('2d')
  if (g) {
    g.imageSmoothingQuality = 'high'
    g.drawImage(src, 0, 0, w, h, 0, 0, out.width, out.height)
  }
  return out
}

/** The pixel roster, stern to bow: the flag aft, the captain amidships, the lookout at the bow. */
export const ROSTER: { kind: Kind; color: string }[] = [
  { kind: 'flag', color: 'sage' },
  { kind: 'bow', color: 'mustard' },
  { kind: 'pirate', color: 'orange' },
  { kind: 'headphones', color: 'plum' },
  { kind: 'waver', color: 'slate' },
  { kind: 'spyglass', color: 'blue' },
]

export type Role = 'waver' | 'lookout' | 'prop' | 'plain'

/** What a member does with its "wave" loop, from its id and accessory. */
export function roleOf(id: string | undefined, accessory?: string): Role {
  const a = `${id ?? ''} ${accessory ?? ''}`.toLowerCase()
  if (a.includes('wav')) return 'waver'
  if (a.includes('spyglass') || a.includes('lookout') || a.includes('telescope')) return 'lookout'
  if (a.includes('flag') || a.includes('pennant')) return 'prop'
  return 'plain'
}

/**
 * A light edge around painted art for the dark theme (the kit's ink nearly vanishes on dark
 * paper): the sprite's silhouette dilated by `r` device px, tinted, under the sprite. Done once.
 */
export function withEdge(src: HTMLCanvasElement, r: number, color: string, alpha: number): HTMLCanvasElement {
  const w = src.width
  const h = src.height
  const mask = document.createElement('canvas')
  mask.width = w
  mask.height = h
  const out = document.createElement('canvas')
  out.width = w
  out.height = h
  const mg = mask.getContext('2d')
  const og = out.getContext('2d')
  if (!mg || !og) return src
  const n = 12
  for (let i = 0; i < n; i++) {
    const a = (i / n) * Math.PI * 2
    mg.drawImage(src, Math.cos(a) * r, Math.sin(a) * r)
  }
  mg.globalCompositeOperation = 'source-in'
  mg.fillStyle = color
  mg.fillRect(0, 0, w, h)
  og.globalAlpha = alpha
  og.drawImage(mask, 0, 0)
  og.globalAlpha = 1
  og.drawImage(src, 0, 0)
  return out
}

function mulberry32(seed: number) {
  let a = seed | 0
  return () => {
    a = (a + 0x6d2b79f5) | 0
    let t = Math.imul(a ^ (a >>> 15), 1 | a)
    t = (t + Math.imul(t ^ (t >>> 7), 61 | t)) ^ t
    return ((t ^ (t >>> 14)) >>> 0) / 4294967296
  }
}

const ONE_SHOT: Record<Anim, number> = { idle: 12, blink: 6, look: 12, wave: 12, squash: 6, cheer: 12 }

/** One crew member's animation state: which anim is playing, since when, and what comes next. */
export class Animator {
  role: Role
  rand: () => number
  base: Anim = 'idle'
  anim: Anim = 'idle'
  start = 0
  /** End time of the current one-shot (or Infinity while on the base loop). */
  end = Infinity
  baseStart: number
  nextBlink: number
  nextLook: number
  nextCheer: number
  nextHop: number
  nextGesture: number
  nextBase: number
  gestureArm = 0
  gestureEnd = -1
  lookDir = 1
  breathOff: number
  /** The physics wants a hop at this time (Sea.tsx starts it). */
  hopDue = false
  fps: number
  frames: Record<Anim, number>
  /** Pixel crew: blinks and glances may play over a wave loop (nothing to snap). */
  pixel: boolean

  constructor(seed: number, role: Role, t: number, fps: number, frames?: Partial<Record<Anim, number>>, pixel = false) {
    this.role = role
    this.pixel = pixel
    this.rand = mulberry32(seed)
    const r = this.rand
    this.fps = fps
    this.frames = { ...ONE_SHOT, ...frames }
    this.baseStart = t - r() * 4
    this.breathOff = r() * 4
    this.nextBlink = t + 0.8 + r() * 3.5
    this.nextLook = t + 2.5 + r() * 6
    this.nextCheer = t + 12 + r() * 22
    this.nextHop = t + 3 + r() * 8
    this.nextGesture = t + 2 + r() * 5
    this.nextBase = t + (role === 'plain' ? Infinity : 4 + r() * 6)
    if (role !== 'plain') {
      this.base = this.anim = 'wave'
      this.start = this.baseStart
    }
  }

  private busy(t: number) {
    return t < this.end
  }

  play(anim: Anim, t: number, duration?: number) {
    this.anim = anim
    this.start = t
    // A cheer ends on a little take: hold its last frame a beat before going back.
    this.end = t + (duration ?? this.frames[anim] / this.fps + (anim === 'cheer' ? 0.12 : 0))
  }

  /** One-shots (blink, look, cheer) only interrupt plain idle, so props never snap mid-loop. */
  private calm(t: number) {
    return !this.busy(t) && (this.base === 'idle' || this.pixel)
  }

  /** Greet the pointer: a short wave if nothing else is going on. */
  greet(t: number) {
    if (this.role !== 'plain' || this.busy(t)) return
    this.play('wave', t, 1.6 + this.rand() * 0.8)
  }

  /** After coming about: a cheer, sometimes with a hop. Waits out anything already playing. */
  celebrate(t: number) {
    if (this.role === 'lookout' && this.base === 'wave' && !this.pixel) return
    this.base = 'idle'
    this.baseStart = t
    this.nextBase = Math.max(this.nextBase, t + 3)
    this.play('cheer', t)
    if (this.rand() < 0.5) this.nextHop = Math.min(this.nextHop, t + 0.3)
  }

  landed(t: number) {
    // The lookout keeps the spyglass at its eye through a landing.
    if (this.role === 'lookout' && this.base === 'wave' && !this.pixel) return
    this.play('squash', t)
  }

  update(t: number, air: boolean) {
    const r = this.rand
    if (!this.busy(t) && this.end !== Infinity) {
      this.anim = this.base
      this.end = Infinity
      this.start = this.baseStart
    }
    if (t >= this.nextBase) {
      // Swap between the accessory loop and plain idle in long stretches.
      const toWave = this.base !== 'wave'
      this.base = toWave ? 'wave' : 'idle'
      this.baseStart = t
      const long = this.role === 'lookout' ? [14, 10, 4, 3] : this.role === 'waver' ? [5, 4, 2.5, 2] : [5, 4, 3.5, 3]
      this.nextBase = t + (toWave ? long[0] + r() * long[1] : long[2] + r() * long[3])
      if (!this.busy(t)) {
        this.anim = this.base
        this.start = t
        this.end = Infinity
      }
    }
    if (t >= this.nextCheer) {
      this.nextCheer = t + 16 + r() * 26
      if (this.calm(t) && !air) {
        this.play('cheer', t)
        if (r() < 0.6) this.nextHop = Math.min(this.nextHop, t + 0.25)
      }
    }
    if (t >= this.nextLook) {
      this.nextLook = t + 5 + r() * 8
      if (this.calm(t)) {
        this.lookDir = r() < 0.5 ? -1 : 1
        this.play('look', t)
      } else this.nextLook = t + 1
    }
    if (t >= this.nextBlink) {
      this.nextBlink = t + (r() < 0.2 ? 0.35 : 2.4 + r() * 4)
      if (this.calm(t)) this.play('blink', t)
      else this.nextBlink = t + 0.6
    }
    if (t >= this.nextHop) {
      this.nextHop = t + (this.role === 'lookout' ? 14 : 6) + r() * 10
      if (!air && this.anim !== 'squash') this.hopDue = true
    }
    if (t >= this.nextGesture) {
      this.nextGesture = t + 3 + r() * 6
      this.gestureArm = r() < 0.5 ? 0 : 1
      this.gestureEnd = t + 0.5 + r() * 0.7
    }
  }

  /** Frame index into the current anim's row. */
  frame(t: number, frames: number, loop: boolean): number {
    const f = Math.floor((t - this.start) * this.fps)
    return loop ? ((f % frames) + frames) % frames : Math.max(0, Math.min(frames - 1, f))
  }

  /** The pixel crew's pose for this moment. */
  pose(kind: Kind, t: number, air: boolean, rising: boolean, lookTo: number): Pose {
    const fps = this.fps
    const f = Math.floor((t - this.start) * fps)
    const p: Pose = { lift: 0, eyes: EYE_C, armL: ARM_REST, armR: ARM_REST, alt: 0, prop: 0 }
    // Breathing: a slow cycle, in for about 40% of it.
    const ph = ((t + this.breathOff) % 3.8) / 3.8
    p.lift = ph > 0.08 && ph < 0.48 ? 1 : 0
    if (air) p.lift = rising ? 1 : 0
    if (lookTo) p.eyes = lookTo < 0 ? EYE_L : EYE_R
    if (t < this.gestureEnd && !air) {
      if (this.gestureArm === 0) p.armL = ARM_LIFT
      else p.armR = ARM_LIFT
    }
    const wave = this.anim === 'wave' || (this.base === 'wave' && (this.anim === 'blink' || this.anim === 'look'))
    if (wave) {
      if (kind === 'flag') {
        p.prop = 1
        p.armL = ARM_LIFT
      } else if (kind === 'spyglass') {
        p.prop = 1
        p.armR = ARM_LIFT
      } else {
        p.armR = ARM_OUT
        p.alt = Math.floor(t * 4) % 2
      }
    }
    if (kind === 'flag') p.alt = Math.floor(t * (p.prop ? 7 : 3.5)) % 2
    switch (this.anim) {
      case 'blink':
        if (f >= 1 && f <= 2) p.eyes = EYE_SHUT
        break
      case 'look':
        if (f >= 1 && f <= 5) p.eyes = this.lookDir < 0 ? EYE_L : EYE_R
        else if (f >= 6 && f <= 10) p.eyes = this.lookDir < 0 ? EYE_R : EYE_L
        break
      case 'cheer': {
        p.eyes = EYE_HAPPY
        const a = Math.floor(f / 2) % 2
        if (kind !== 'flag') p.armL = a ? ARM_UP : ARM_OUT
        if (kind !== 'spyglass') p.armR = a ? ARM_OUT : ARM_UP
        p.alt = a
        break
      }
      case 'squash':
        if (f <= 1) {
          p.lift = -1
          p.eyes = EYE_SHUT
        } else if (f === 2) p.lift = 1
        else p.lift = 0
        break
      default:
        break
    }
    return p
  }
}
