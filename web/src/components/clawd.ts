// The pixel crew (fallback when the painted sheets aren't available): Clawd as in Claude Code's
// splash logo, a 12 x 8 grid of square cells, eyes black.
//
//   ..########..     head
//   ..#O####O#..     eyes at columns 3 and 8
//   ############     arms (columns 0-1 and 10-11) and core
//   ############
//   ..########..     torso
//   ..########..
//   ..#.#..#.#..     four legs
//   ..#.#..#.#..
//
// Poses are discrete frames on that grid: a one-row stretch when breathing in, a one-row squash
// (wider, shorter) on landing, eyes centred / glancing a cell left or right / shut, arms at rest,
// lifted, up or waving, and per-kind accessories on the same grid. Each pose is painted once into
// a small canvas at an integer device-pixel cell size and drawn unscaled, so edges stay sharp.

export type Kind = 'flag' | 'bow' | 'pirate' | 'headphones' | 'waver' | 'spyglass'

export const ARM_REST = 0
export const ARM_LIFT = 1
export const ARM_UP = 2
export const ARM_OUT = 3

export const EYE_C = 0
export const EYE_L = 1
export const EYE_R = 2
export const EYE_SHUT = 3
export const EYE_HAPPY = 4

export interface Pose {
  /** +1 breathing in (one row taller), 0, -1 squashed. */
  lift: number
  eyes: number
  armL: number
  armR: number
  /** Accessory flutter frame. */
  alt: number
  /** Accessory in use (spyglass raised, flag held high). */
  prop: number
}

/** Cell box of every pose (accessories included) and the feet anchor, in cells. */
export const BOX = { x: -4, y: -8, w: 22, h: 16 }
export const FEET = { x: 6, y: 8 }

export interface Palette {
  body: string
  eye: string
  hat: string
  skull: string
  pink: string
  pinkDeep: string
  gear: string
  cushion: string
  brass: string
  lens: string
  flag: string
}

type Paint = (x: number, y: number, color: string) => void

const range = (a: number, b: number) => Array.from({ length: b - a + 1 }, (_, i) => a + i)

function armCells(state: number, alt: number): [number, number][] {
  // Left arm in body-local cells (the right arm mirrors it: x -> 11 - x).
  switch (state) {
    case ARM_LIFT:
      return [
        [0, 1],
        [1, 1],
        [0, 2],
        [1, 2],
      ]
    case ARM_UP:
      return [
        [0, 0],
        [1, 0],
        [0, 1],
        [1, 1],
      ]
    case ARM_OUT:
      // Waving: the arm reaches up and out, swinging between two frames.
      return alt
        ? [
            [0, -1],
            [1, -1],
            [0, 0],
            [1, 0],
          ]
        : [
            [-1, -1],
            [0, -1],
            [0, 0],
            [1, 0],
          ]
    default:
      return [
        [0, 2],
        [1, 2],
        [0, 3],
        [1, 3],
      ]
  }
}

function compose(kind: Kind, pose: Pose, pal: Palette, paint: Paint, lid: (x: number, y: number, happy: boolean) => void) {
  const lift = pose.lift
  const up = (y: number) => y - lift
  const squash = lift < 0
  const body = pal.body
  // Legs stay planted; the torso stretches or squashes between them and the upper body.
  for (const x of [2, 4, 7, 9]) {
    paint(x, 6, body)
    paint(x, 7, body)
  }
  for (let y = up(4); y <= 5; y++) for (const x of range(squash ? 1 : 2, squash ? 10 : 9)) paint(x, y, body)
  for (const y of [0, 1]) for (const x of range(2, 9)) paint(x, up(y), body)
  for (const y of [2, 3]) for (const x of range(squash ? 1 : 2, squash ? 10 : 9)) paint(x, up(y), body)
  const wide = squash ? 1 : 0
  for (const [x, y] of armCells(pose.armL, pose.alt)) paint(x - wide, up(y), body)
  for (const [x, y] of armCells(pose.armR, 1 - pose.alt)) paint(11 - x + wide, up(y), body)

  // Eyes: a cell each; glancing moves both a cell; shut or happy is a lid line.
  const shift = pose.eyes === EYE_L ? -1 : pose.eyes === EYE_R ? 1 : 0
  const eyeL = 3 + shift
  const eyeR = 8 + shift
  const patch = kind === 'pirate'
  const peering = kind === 'spyglass' && pose.prop
  const showL = !patch
  if (pose.eyes === EYE_SHUT || pose.eyes === EYE_HAPPY) {
    if (showL) lid(eyeL, up(1), pose.eyes === EYE_HAPPY)
    if (!peering) lid(eyeR, up(1), pose.eyes === EYE_HAPPY)
  } else {
    // Peering through the spyglass: the free eye squints.
    if (showL) {
      if (peering) lid(3, up(1), false)
      else paint(eyeL, up(1), pal.eye)
    }
    if (!peering) paint(eyeR, up(1), pal.eye)
  }

  switch (kind) {
    case 'pirate': {
      // Tricorn: a wide brim with upturned tips, a crown with a skull in it, and a patch over the
      // left eye on a strap.
      for (const x of range(1, 10)) paint(x, up(-1), pal.hat)
      paint(0, up(-2), pal.hat)
      paint(11, up(-2), pal.hat)
      paint(1, up(-2), pal.hat)
      paint(10, up(-2), pal.hat)
      for (const x of range(3, 8)) paint(x, up(-2), pal.hat)
      for (const x of range(3, 8)) paint(x, up(-3), pal.hat)
      for (const x of range(4, 7)) paint(x, up(-4), pal.hat)
      paint(5, up(-3), pal.skull)
      paint(6, up(-3), pal.skull)
      paint(5, up(-2), pal.skull)
      paint(6, up(-2), pal.skull)
      paint(2, up(1), pal.hat)
      paint(3, up(1), pal.hat)
      paint(2, up(2), pal.hat)
      paint(3, up(2), pal.hat)
      paint(4, up(0), pal.hat)
      break
    }
    case 'bow': {
      // A pink bow on the right of the head: two loops and a knot.
      for (const [x, y] of [
        [6, -2],
        [7, -2],
        [6, -1],
        [7, -1],
        [10, -2],
        [11, -2],
        [10, -1],
        [11, -1],
      ] as const)
        paint(x - 1, up(y), pal.pink)
      paint(7, up(-1), pal.pinkDeep)
      paint(8, up(-1), pal.pinkDeep)
      break
    }
    case 'headphones': {
      for (const x of range(3, 8)) paint(x, up(-2), pal.gear)
      paint(2, up(-1), pal.gear)
      paint(9, up(-1), pal.gear)
      for (const y of [0, 1]) {
        paint(1, up(y), pal.gear)
        paint(2, up(y), pal.cushion)
        paint(9, up(y), pal.cushion)
        paint(10, up(y), pal.gear)
      }
      break
    }
    case 'spyglass': {
      if (pose.prop) {
        // Held to the right eye, out over the bow: narrow eyepiece, wider barrel, lens.
        for (const x of [9, 10]) paint(x, up(1), pal.brass)
        for (const x of [11, 12, 13]) {
          paint(x, up(0), pal.brass)
          paint(x, up(1), pal.brass)
        }
        paint(14, up(0), pal.lens)
        paint(14, up(1), pal.lens)
      } else {
        // Lowered, tucked under the arm.
        for (const x of [11, 12, 13]) paint(x, up(4), pal.brass)
        paint(14, up(4), pal.lens)
      }
      break
    }
    case 'flag': {
      // A little flag on a pole held by the left arm, flying aft-to-fore like the pennant.
      const top = pose.prop ? -8 : -7
      for (let y = top; y <= 3; y++) paint(-1, up(y), pal.gear)
      const a = pose.alt
      for (const x of range(0, a ? 3 : 4)) paint(x, up(top), pal.flag)
      for (const x of range(0, a ? 4 : 3)) paint(x, up(top + 1), pal.flag)
      paint(0, up(top + 2), pal.flag)
      if (a) paint(1, up(top + 2), pal.flag)
      break
    }
    default:
      break
  }
}

const cache = new Map<string, HTMLCanvasElement>()

export function clearClawdCache() {
  cache.clear()
}

/** A pose painted at `cell` device pixels per grid cell; feet at BOX-relative FEET. */
export function clawdFrame(kind: Kind, pose: Pose, pal: Palette, cell: number): HTMLCanvasElement | null {
  const key = `${kind}|${pal.body}|${cell}|${pose.lift}${pose.eyes}${pose.armL}${pose.armR}${pose.alt}${pose.prop}`
  const hit = cache.get(key)
  if (hit) return hit
  const c = document.createElement('canvas')
  c.width = BOX.w * cell
  c.height = BOX.h * cell
  const g = c.getContext('2d')
  if (!g) return null
  const paint: Paint = (x, y, color) => {
    g.fillStyle = color
    g.fillRect((x - BOX.x) * cell, (y - BOX.y) * cell, cell, cell)
  }
  const lid = (x: number, y: number, happy: boolean) => {
    // A shut eye is a thin dark bar across the lower half of the eye cell; a happy one is a
    // little arch (two half-cells up, the middle higher).
    const t = Math.max(1, Math.round(cell * 0.3))
    g.fillStyle = pal.eye
    const px = (x - BOX.x) * cell
    const py = (y - BOX.y) * cell
    if (happy) {
      const half = Math.max(1, Math.floor(cell / 2))
      g.fillRect(px, py + half, Math.max(1, Math.floor(cell / 3)), t)
      g.fillRect(px + cell - Math.max(1, Math.floor(cell / 3)), py + half, Math.max(1, Math.floor(cell / 3)), t)
      g.fillRect(px + Math.floor(cell / 4), py + half - t, cell - 2 * Math.floor(cell / 4), t)
    } else {
      g.fillRect(px, py + Math.round(cell * 0.55), cell, t)
    }
  }
  compose(kind, pose, pal, paint, lid)
  cache.set(key, c)
  if (cache.size > 400) cache.delete(cache.keys().next().value as string)
  return c
}
