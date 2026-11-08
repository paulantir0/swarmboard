import { useEffect, useRef, type RefObject } from 'react'
import { BOAT_H, BOAT_TOP, BOAT_W, Boat, BoatRail, DECK_MID, DECK_X0, DECK_X1, WATERLINE, deckY, hullSamples } from './Boat'
import { BOX, FEET, clawdFrame, clearClawdCache, type Kind, type Palette } from './clawd'
import { ANIMS, Animator, ROSTER, downscale, loadAssets, roleOf, withEdge, type Anim, type Assets } from './crew'
import { BoatSim, CrewSim, STEP, type Course, type Deck, type HullDef } from './physics'
import { SNIPPETS, isMarker } from './seaText'
import { FOAM_RING, LUT_DX, NW, makeBand, resetSpring, springAt, springSlope, stepSpring, type Band } from './water'

// A side-view sea of code-review noise. Four bands of text are the water: every glyph is a
// particle on a Gerstner sea (water.ts), placed with sub-pixel precision and leaning to the local
// slope; rows deeper down orbit less. Steep crests throw up foam glyphs that linger on the water
// and fade. A spring surface on each band carries the boat's bow wave and wake, and ripples from
// the pointer. A rigid-body boat floats on its band by buoyancy (physics.ts); its crew ride on
// sprung feet along the deck and hop under gravity.
//
// Layers: back canvas (the bands behind the boat), the SVG boat, a canvas for the crew (and the
// painted boat, when its sheet exists), the SVG rail, then the front canvas: the boat's own band,
// whose slightly translucent water cuts the hull at the waterline, and the bands in front of it.
//
// Physics runs at a fixed 120 Hz and is interpolated for drawing; the sea is drawn every
// animation frame from the simulation clock, which stops while the tab is hidden. Under reduced
// motion one settled still frame is drawn.

const FAMILY = "'Swarm Mono', ui-monospace, monospace"
const BANDS = 4
const SET_BODY = 0
const SET_CREST = 1
const SET_ACCENT = 2
const BODY_INK = 0.74
const FOAM_CHARS = ['·', '°', '˜', '≈', '·', '˜']
const MAX_ROWS = 40

const lerp = (a: number, b: number, t: number) => a + (b - a) * t
const clamp = (v: number, a: number, b: number) => Math.max(a, Math.min(b, v))

// Glyph table: every character in the snippets, plus foam. Index < 128, so the high bit marks accent.
const CHARS = [...new Set([' ', ...SNIPPETS.join(''), ...FOAM_CHARS])]
const INDEX = new Map(CHARS.map((c, i) => [c, i]))
const FOAM_IDX = FOAM_CHARS.map((c) => INDEX.get(c) ?? 0)
const ACCENT = 128

function mulberry32(seed: number) {
  let a = seed | 0
  return () => {
    a = (a + 0x6d2b79f5) | 0
    let t = Math.imul(a ^ (a >>> 15), 1 | a)
    t = (t + Math.imul(t ^ (t >>> 7), 61 | t)) ^ t
    return ((t ^ (t >>> 14)) >>> 0) / 4294967296
  }
}

/** Integer hash to [0, 1). */
function hash(n: number) {
  let x = Math.imul(n ^ 0x9e3779b9, 0x85ebca6b)
  x = Math.imul(x ^ (x >>> 13), 0xc2b2ae35)
  return ((x ^ (x >>> 16)) >>> 0) / 4294967296
}

/** One row of text, at least `min` characters long, wrapping seamlessly. */
function rowText(rand: () => number, min: number): Uint8Array {
  const out: number[] = []
  while (out.length < min) {
    const s = SNIPPETS[Math.floor(rand() * SNIPPETS.length)]
    const accent = isMarker(s) ? ACCENT : 0
    for (const ch of s) out.push((INDEX.get(ch) ?? 0) | accent)
    const gap = 3 + Math.floor(rand() * 5)
    for (let g = 0; g < gap; g++) out.push(0)
  }
  return Uint8Array.from(out)
}

/** The boat and crew as laid out for this viewport (CSS px; local = relative to the pivot). */
interface Rig {
  painted: boolean
  w: number
  h: number
  /** Pivot (waterline centre) within the boat box. */
  ox: number
  oy: number
  /** Art extents relative to the pivot: masthead height, and left / right edges. */
  top: number
  left: number
  right: number
  /** Left and right edges of the art at local height y (heading right), or null if none. */
  ext: (y: number) => [number, number] | null
  hull: HullDef
  deck: Deck
  slots: number[]
  maxSway: number
  /** Crew: painted scale (CSS px per frame px) or pixel cell (device px). */
  crewPainted: boolean
  crewScale: number
  cell: number
  crewH: number
}

interface Pointer {
  x: number
  y: number
  t: number
  /** Last time it moved, in sim seconds. */
  sim: number
  queued: { x: number; y: number; dv: number }[]
}

export function Sea({ sky }: { sky: RefObject<HTMLElement | null> }) {
  const backRef = useRef<HTMLCanvasElement>(null)
  const frontRef = useRef<HTMLCanvasElement>(null)
  const crewRef = useRef<HTMLCanvasElement>(null)
  const boatRef = useRef<HTMLDivElement>(null)
  const railRef = useRef<HTMLDivElement>(null)

  useEffect(() => {
    const back = backRef.current
    const front = frontRef.current
    const crewCv = crewRef.current
    const boatEl = boatRef.current
    const railEl = railRef.current
    if (!back || !front || !crewCv || !boatEl || !railEl) return
    const bctx = back.getContext('2d')
    const fctx = front.getContext('2d')
    const cctx = crewCv.getContext('2d')
    if (!bctx || !fctx || !cctx) return

    const root = document.documentElement
    const reduce = window.matchMedia('(prefers-reduced-motion: reduce)')
    const scheme = window.matchMedia('(prefers-color-scheme: dark)')
    const isDark = () => root.dataset.theme === 'dark' || (root.dataset.theme !== 'light' && scheme.matches)

    let W = 0
    let H = 0
    let dpr = 1
    let lineH = 18
    let bands: Band[] = []
    let atlas: HTMLCanvasElement | null = null
    let cw = 8 // atlas cell, device px
    let ch = 16
    let step = 8 // glyph advance, CSS px
    let paper = '#ffffff'
    let cols = 0
    let small = false

    // Assets (painted crew and boat) and the rig built from them or from the fallbacks.
    let assets: Assets | null = null
    let assetsSettled = false
    let rig: Rig | null = null
    let boatBand = -1
    let course: Course = { xMin: 0, xMax: 0, hold: 0, cruise: 0, current: 0 }
    let boat: BoatSim | null = null
    let turnsSeen = 0
    let crewSims: CrewSim[] = []
    let animators: Animator[] = []
    let kinds: Kind[] = []
    let pal: Palette | null = null
    let crewSheets: HTMLCanvasElement[] = []
    let crewCell = { w: 0, h: 0 }
    let boatSheet: HTMLCanvasElement | null = null
    let boatCell = { w: 0, h: 0 }
    type Box = { left: number; top: number; right: number; bottom: number }
    let boatArt: Box | null = null
    /** Per-row left / right alpha edges of the painted boat (frame px), every `rowStep` px. */
    let boatRows: { l: Float32Array; r: Float32Array; step: number } | null = null
    /** Width of a painted crew sprite with its arms out (frame px). */
    let crewArtW = 0

    // Clock: simulation time advances only while animating.
    let simT = 7
    let acc = 0
    let frameDt = 0
    const ptr: Pointer = { x: -1e4, y: -1e4, t: 0, sim: -100, queued: [] }
    let lastGreet = -100

    const readPalette = (): Palette => {
      const css = getComputedStyle(root)
      const v = (n: string, d: string) => css.getPropertyValue(n).trim() || d
      return {
        body: '#da7758',
        eye: v('--crew-eye', '#000000'),
        hat: v('--crew-hat', '#1a1a1a'),
        skull: v('--crew-skull', '#ffffff'),
        pink: v('--crew-pink', '#ef8fb4'),
        pinkDeep: v('--crew-pink-deep', '#cf5f8c'),
        gear: v('--crew-gear', '#2d2c29'),
        cushion: v('--crew-cushion', '#55534d'),
        brass: v('--crew-brass', '#b8893a'),
        lens: v('--crew-lens', '#9cc3d9'),
        flag: v('--crew-flag', '#d6453a'),
      }
    }
    let bodyColors: string[] = []

    const buildAtlas = (size: number) => {
      const css = getComputedStyle(root)
      const ink = css.getPropertyValue('--sea-ink').trim() || '17, 17, 17'
      const accent = css.getPropertyValue('--sea-accent').trim() || '44, 74, 124'
      paper = css.getPropertyValue('--color-canvas').trim() || '#ffffff'
      cw = Math.max(2, Math.round(size * 0.6 * dpr))
      ch = Math.ceil(size * 1.4 * dpr)
      step = cw / dpr
      const img = document.createElement('canvas')
      img.width = cw * CHARS.length
      img.height = ch * 3
      const g = img.getContext('2d')
      if (!g) return null
      g.font = `400 ${size * dpr}px ${FAMILY}`
      g.textAlign = 'center'
      g.textBaseline = 'middle'
      const sets: [string, number][] = [
        [ink, BODY_INK],
        [ink, 1],
        [accent, 0.9],
      ]
      sets.forEach(([rgb, a], set) => {
        g.fillStyle = `rgba(${rgb}, ${a})`
        for (let i = 1; i < CHARS.length; i++) g.fillText(CHARS[i], i * cw + cw / 2, set * ch + ch / 2)
      })
      return img
    }

    /** Alpha bounds of one cell of a sheet, in source px. */
    const scanAlpha = (img: HTMLImageElement, sx: number, sy: number, sw: number, sh: number): Box => {
      const k = Math.min(1, 320 / sw)
      const w = Math.max(1, Math.round(sw * k))
      const h = Math.max(1, Math.round(sh * k))
      const c = document.createElement('canvas')
      c.width = w
      c.height = h
      const g = c.getContext('2d', { willReadFrequently: true })
      const none = { left: 0, top: 0, right: sw, bottom: sh }
      if (!g) return none
      g.drawImage(img, sx, sy, sw, sh, 0, 0, w, h)
      try {
        const d = g.getImageData(0, 0, w, h).data
        let l = w
        let r = -1
        let tp = h
        let bt = -1
        for (let y = 0; y < h; y++)
          for (let x = 0; x < w; x++)
            if (d[(y * w + x) * 4 + 3] > 24) {
              if (x < l) l = x
              if (x > r) r = x
              if (y < tp) tp = y
              if (y > bt) bt = y
            }
        if (r < 0) return none
        return { left: l / k, top: tp / k, right: (r + 1) / k, bottom: (bt + 1) / k }
      } catch {
        return none
      }
    }

    /** Left / right alpha edges of every row of one cell, in source px. */
    const scanRows = (img: HTMLImageElement, sw: number, sh: number) => {
      const k = Math.min(1, 320 / sw)
      const w = Math.max(1, Math.round(sw * k))
      const h = Math.max(1, Math.round(sh * k))
      const c = document.createElement('canvas')
      c.width = w
      c.height = h
      const g = c.getContext('2d', { willReadFrequently: true })
      if (!g) return null
      g.drawImage(img, 0, 0, sw, sh, 0, 0, w, h)
      try {
        const d = g.getImageData(0, 0, w, h).data
        const l = new Float32Array(h).fill(-1)
        const r = new Float32Array(h).fill(-1)
        for (let y = 0; y < h; y++)
          for (let x = 0; x < w; x++)
            if (d[(y * w + x) * 4 + 3] > 24) {
              if (l[y] < 0) l[y] = x / k
              r[y] = (x + 1) / k
            }
        return { l, r, step: 1 / k }
      } catch {
        return null
      }
    }

    /** Build the rig at scene scale `f`, or null if it can't be built. */
    const makeRig = (f: number): Rig | null => {
      const man = assets?.manifest
      const pb = assets?.boat && man?.boat ? man.boat : null
      const pc = assets?.crew && man?.crew ? man.crew : null
      const n = pc ? pc.members.length : ROSTER.length
      // Crew size at this scale.
      const baseCrewH = pc ? pc.display.height * (small ? 0.78 : 1) : 0
      const crewScale = pc ? (baseCrewH * f) / pc.frame.h : 0
      const minCell = Math.ceil((small ? 2.25 : 3.5) * dpr)
      const cell = Math.max(minCell, Math.round((small ? 2.5 : 4) * f * dpr))
      const cellCss = cell / dpr
      const crewW = pc ? (crewArtW || pc.frame.w * 0.85) * crewScale * 0.97 : 12 * cellCss
      const crewH = pc ? pc.frame.h * crewScale * 0.6 : 8 * cellCss
      if (pb) {
        const art = boatArt ?? { left: 0, top: 0, right: pb.frame.w, bottom: pb.frame.h }
        let slotGap = Infinity
        for (let i = 1; i < pb.slots.length; i++) slotGap = Math.min(slotGap, Math.abs(pb.slots[i] - pb.slots[i - 1]))
        // Wide enough that the crew, at their manifest size, don't crowd the deck.
        const want = pc && Number.isFinite(slotGap) ? ((crewArtW || pc.frame.w * 0.85) * 0.97 * (pc.display.height / pc.frame.h)) / slotGap : 0
        const wide = Math.max(pb.display.width / pb.frame.w, want)
        const base = Math.min(wide, (W - 16) / (art.right - art.left))
        const bs = base * f
        const ox = (pb.frame.w / 2) * bs
        const oy = pb.waterline * bs
        const [[dx0, dy0], [dx1, dy1]] = pb.deck
        const deckAt = (fx: number) => (dx1 === dx0 ? dy0 : dy0 + ((dy1 - dy0) * (fx - dx0)) / (dx1 - dx0))
        const hp = pb.hull
        const pts = new Float32Array(hp.length * 2)
        const cap = new Float32Array(hp.length)
        hp.forEach(([hx, hy], i) => {
          pts[2 * i] = (hx - pb.frame.w / 2) * bs
          pts[2 * i + 1] = (hy - pb.waterline) * bs
          cap[i] = Math.max(20, hy - deckAt(hx) + 40) * bs
        })
        const slotsF = pb.slots.slice(0, n)
        const slots = slotsF.map((x) => (x - pb.frame.w / 2) * bs)
        let gap = Infinity
        for (let i = 1; i < slots.length; i++) gap = Math.min(gap, Math.abs(slots[i] - slots[i - 1]))
        if (!Number.isFinite(gap)) gap = crewW * 2
        // Pixel crew on the painted boat: sized to the slot pitch.
        const pcell = pc ? cell : Math.max(Math.ceil(2 * dpr), Math.min(cell, Math.floor(((gap * 0.82) / 12) * dpr)))
        // Painted crew: their manifest size, unless the slots are closer than that (small screens).
        const pscale = pc ? Math.min(crewScale, (gap * 1.03) / (crewArtW || pc.frame.w * 0.85)) : 0
        return {
          painted: true,
          w: pb.frame.w * bs,
          h: pb.frame.h * bs,
          ox,
          oy,
          top: (pb.waterline - art.top) * bs,
          left: (art.left - pb.frame.w / 2) * bs,
          right: (art.right - pb.frame.w / 2) * bs,
          ext: (ly: number) => {
            const rows = boatRows
            if (!rows) return [(art.left - pb.frame.w / 2) * bs, (art.right - pb.frame.w / 2) * bs]
            const i = Math.floor((ly / bs + pb.waterline) / rows.step)
            if (i < 0 || i >= rows.l.length || rows.l[i] < 0) return null
            return [(rows.l[i] - pb.frame.w / 2) * bs, (rows.r[i] - pb.frame.w / 2) * bs]
          },
          hull: {
            pts,
            cap,
            length: Math.abs(hp[0][0] - hp[hp.length - 1][0]) * bs,
            bowX: pts[0],
            sternX: pts[pts.length - 2],
            ce: (pb.waterline - art.top) * bs * 0.4,
          },
          deck: { y: (lx: number) => (deckAt(lx / bs + pb.frame.w / 2) - pb.waterline) * bs },
          slots,
          maxSway: Math.max(2, gap * 0.16),
          crewPainted: !!pc,
          crewScale: pscale,
          cell: pcell,
          crewH: pc ? pc.frame.h * pscale * 0.6 : 8 * (pcell / dpr),
        }
      }
      // SVG boat, sized so the crew fit on the deck.
      const pitch = pc ? crewW : 14 * cellCss
      const span = (n - 1) * pitch + crewW + (pc ? crewW * 0.2 : 4 * cellCss)
      const bs = span / (DECK_X1 - DECK_X0)
      const samples = hullSamples(11)
      const pts = new Float32Array(samples.length * 2)
      const cap = new Float32Array(samples.length)
      samples.forEach(([x, y, c], i) => {
        pts[2 * i] = (x - BOAT_W / 2) * bs
        pts[2 * i + 1] = (y - WATERLINE) * bs
        cap[i] = c * bs
      })
      return {
        painted: false,
        w: BOAT_W * bs,
        h: BOAT_H * bs,
        ox: (BOAT_W / 2) * bs,
        oy: WATERLINE * bs,
        top: (WATERLINE - BOAT_TOP) * bs,
        left: (30 - BOAT_W / 2) * bs,
        right: (395 - BOAT_W / 2) * bs,
        ext: (ly: number) => {
          // Rigging and sails narrow linearly to the masthead; below the boom it's the hull.
          const y = ly / bs + WATERLINE
          if (y < BOAT_TOP) return null
          const l = y >= 145 ? 30 : 34 + ((145 - y) / 128) * 164
          const r = y >= 131 ? 395 : 392 - ((131 - y) / 114) * 191
          return [(l - BOAT_W / 2) * bs, (r - BOAT_W / 2) * bs]
        },
        hull: { pts, cap, length: (362 - 40) * bs, bowX: pts[0], sternX: pts[pts.length - 2], ce: (WATERLINE - 80) * bs },
        deck: { y: (lx: number) => (deckY(lx / bs + BOAT_W / 2) - WATERLINE) * bs },
        slots: Array.from({ length: n }, (_, k) => (DECK_MID - BOAT_W / 2) * bs + (k - (n - 1) / 2) * pitch),
        maxSway: pitch * 0.14,
        crewPainted: !!pc,
        crewScale,
        cell,
        crewH,
      }
    }

    const layout = () => {
      const rect = back.getBoundingClientRect()
      W = Math.max(1, Math.round(rect.width))
      H = Math.max(1, Math.round(rect.height))
      dpr = Math.min(2, window.devicePixelRatio || 1)
      for (const c of [back, front, crewCv]) {
        c.width = Math.ceil(W * dpr)
        c.height = Math.ceil(H * dpr)
      }
      small = W < 640
      const el = sky.current
      const skyBottom = el ? el.offsetTop + el.offsetHeight : H * 0.4
      const skyRight = el ? el.offsetLeft + el.offsetWidth : W * 0.5
      const size = small ? 11 : 13
      lineH = Math.round(size * 1.38)
      atlas = buildAtlas(size)
      cols = Math.ceil(W / step) + 4

      // Bands: the back crest can reach seaTop, the rest stack evenly to the bottom edge.
      const seaTop = Math.min(H - 120, Math.max(H * 0.45, skyBottom + 10))
      const A = small ? 20 : Math.min(46, Math.max(26, (H - seaTop) * 0.11))
      const base0 = seaTop + A
      const gap = (H - base0) / (BANDS - 0.15)
      const dark = isDark()
      const rand = mulberry32(0x5eab0a7)
      bands = []
      for (let i = 0; i < BANDS; i++) {
        const z = i / (BANDS - 1)
        bands.push(
          makeBand({
            index: i,
            base: base0 + i * gap,
            p: lerp(0.58, 1, z),
            scale: small ? 0.62 : 1,
            amp: A,
            calm: lerp(0.82, 1, z),
            drift: lerp(5, 14, z) * (small ? 0.75 : 1),
            alpha: lerp(0.15, dark ? 0.42 : 0.5, z),
            cols,
            rows: MAX_ROWS,
            width: W,
            lineH,
            rand,
          }),
        )
      }
      for (let i = 0; i < BANDS; i++) {
        const b = bands[i]
        // Rows reach just past the lowest trough of the band in front (anything lower is hidden).
        const bottom = i < BANDS - 1 ? bands[i + 1].base + bands[i + 1].ampMax + lineH : H + lineH
        const n = clamp(Math.ceil((bottom - (b.base - b.ampMax)) / lineH) + 1, 1, MAX_ROWS)
        for (let r = 0; r < n; r++) b.rows.push(rowText(rand, cols * 2 + 60))
      }

      // Boat: the largest scene scale whose masthead clears the copy, on the highest band that
      // allows it. If nothing fits, it stays ashore.
      rig = null
      boatBand = -1
      if (assetsSettled) {
        const scales = [1, 0.93, 0.86, 0.8, 0.74, 0.68, 0.62, 0.57, 0.52]
        const rigs = scales.map((f) => makeRig(f))
        // The run the boat may sail on band i: whatever of it can rise above the bottom of the
        // copy has to stay right of the copy on either tack (with room for pitching); the rest
        // only has to stay on screen.
        const courseOn = (r: Rig, i: number): Course | null => {
          const b = bands[i]
          const yHigh = b.base - b.ampMax - lineH * 0.45 - 6
          const cut = skyBottom + 10 - yHigh
          let upL = Infinity
          let upR = -Infinity
          for (let y = -r.top - 2; y < cut; y += 3) {
            const e = r.ext(y)
            if (!e) continue
            const m = Math.abs(y) * 0.1
            upL = Math.min(upL, e[0] - m)
            upR = Math.max(upR, e[1] + m)
          }
          const edge = small ? 6 : 10
          let xMin = Math.max(edge - r.left, edge + r.right)
          const xMax = Math.min(W - edge - r.right, W - edge + r.left)
          if (upR > -Infinity) xMin = Math.max(xMin, skyRight + 18 - upL, skyRight + 18 + upR)
          if (xMax < xMin) return null
          return {
            xMin: xMin + 8,
            xMax: Math.max(xMin + 8, xMax - 8),
            hold: (xMin + xMax) / 2,
            cruise: clamp((r.right - r.left) * 0.085, 14, 46),
            current: b.drift,
          }
        }
        // Prefer the largest boat on a band with water in front of it and a real run to sail;
        // then any run; the front band (nothing washing over the hull) only as a last resort.
        // Phones care most about crew size, so there any band will do.
        const passes: { last: boolean; run: number }[] = small
          ? [
              { last: true, run: 40 },
              { last: true, run: 0 },
            ]
          : [
              { last: false, run: 200 },
              { last: false, run: 90 },
              { last: true, run: 60 },
              { last: true, run: 0 },
            ]
        search: for (const pass of passes) {
          for (const r of rigs) {
            if (!r || r.right - r.left > W - 8) continue
            for (let i = 1; i < BANDS; i++) {
              if (i === BANDS - 1 && !pass.last) break
              const c = courseOn(r, i)
              if (!c || c.xMax - c.xMin < pass.run) continue
              boatBand = i
              rig = r
              course = c
              break search
            }
          }
        }
      }

      // Physics, crew and their art for this rig.
      boat = null
      crewSims = []
      animators = []
      kinds = []
      pal = readPalette()
      const css = getComputedStyle(root)
      bodyColors = ROSTER.map((m) => css.getPropertyValue(`--crew-${m.color}`).trim() || '#da7758')
      clearClawdCache()
      crewSheets = []
      boatSheet = null
      const showSvg = !!rig && !rig.painted
      for (const layer of [boatEl, railEl]) {
        layer.style.display = showSvg ? '' : 'none'
        if (rig && showSvg) {
          layer.style.width = `${rig.w}px`
          layer.style.height = `${rig.h}px`
          layer.style.transformOrigin = `${rig.ox}px ${rig.oy}px`
        }
      }
      if (!rig || boatBand < 0) return
      const band = bands[boatBand]
      const surf = lineH * 0.45
      boat = new BoatSim(rig.hull, band.g * 2.2, surf)
      turnsSeen = 0
      const man = assets?.manifest
      const members = rig.crewPainted && man?.crew ? man.crew.members : null
      const n = rig.slots.length
      for (let k = 0; k < n; k++) {
        crewSims.push(new CrewSim(rig.slots[k], rig.maxSway))
        const role = members ? roleOf(members[k]?.id, members[k]?.accessory) : roleOf(ROSTER[k % ROSTER.length].kind)
        const frames: Partial<Record<Anim, number>> = {}
        if (members) for (const a of ANIMS) if (members[k].anims[a]) frames[a] = members[k].anims[a].frames
        animators.push(new Animator(0x51a7 + k * 7919, role, simT, man?.fps ?? 12, frames, !members))
        kinds.push(ROSTER[k % ROSTER.length].kind)
      }
      // Pre-scale the painted sheets to the exact device size we draw at. On dark paper they get a
      // faint light edge so the ink outline still reads.
      const edgeRaw = css.getPropertyValue('--crew-edge').trim()
      const edge = edgeRaw && edgeRaw !== 'none' && edgeRaw !== 'transparent' ? edgeRaw : ''
      const edgeR = Math.max(1, 1.25 * dpr)
      if (rig.crewPainted && man?.crew && assets?.crew) {
        const c = man.crew
        crewCell = { w: Math.max(1, Math.round(c.frame.w * rig.crewScale * dpr)), h: Math.max(1, Math.round(c.frame.h * rig.crewScale * dpr)) }
        crewSheets = assets.crew.map((img) => {
          const colsN = Math.max(1, Math.round(img.naturalWidth / c.frame.w))
          const rowsN = Math.max(1, Math.round(img.naturalHeight / c.frame.h))
          const sheet = downscale(img, img.naturalWidth, img.naturalHeight, colsN * crewCell.w, rowsN * crewCell.h)
          return edge ? withEdge(sheet, edgeR, edge, 0.6) : sheet
        })
      }
      if (rig.painted && man?.boat && assets?.boat) {
        const b = man.boat
        boatCell = { w: Math.max(1, Math.round(rig.w * dpr)), h: Math.max(1, Math.round(rig.h * dpr)) }
        const sheet = downscale(assets.boat, b.frame.w * b.frames, b.frame.h, boatCell.w * b.frames, boatCell.h)
        boatSheet = edge ? withEdge(sheet, edgeR, edge, 0.5) : sheet
      }
      for (const b of bands) resetSpring(b)
      // Settle: start at rest on the water a third of the way along the run, heading out, and let
      // it get under way for a few seconds.
      const startX = course.xMax - course.xMin > 24 ? lerp(course.xMin, course.xMax, 0.3) : course.hold
      boat.place(band, startX, simT - 4, 1)
      for (const c of crewSims) c.step(boat, rig.deck, crewG(), STEP, simT - 4)
      for (let t = simT - 4; t < simT; t += STEP) physicsStep(t, false)
      turnsSeen = boat.turns
      acc = 0
    }

    const crewG = () => 520 * (rig ? clamp(rig.crewH / 30, 0.6, 1.3) : 1)
    /** The deck as seen while the boat turns: mirrored and foreshortened with it. */
    let deckSx = 1
    const mirroredDeck: Deck = { y: (lx: number) => (rig ? rig.deck.y(lx / deckSx) : 0) }

    const physicsStep = (t: number, live: boolean) => {
      if (!boat || !rig || boatBand < 0) return
      const band = bands[boatBand]
      boat.disturb(band, 1, band.drift)
      if (boatBand + 1 < BANDS) boat.disturb(bands[boatBand + 1], 0.45, band.drift)
      if (live && ptr.queued.length) {
        for (const q of ptr.queued) poke(q.x, q.y, q.dv)
        ptr.queued.length = 0
      }
      for (const b of bands) stepSpring(b, STEP)
      boat.step(band, t, STEP, course)
      const sxv = boat.sx
      deckSx = (sxv < 0 || (sxv === 0 && boat.dir < 0) ? -1 : 1) * Math.max(0.05, Math.abs(sxv))
      const g = crewG()
      for (let k = 0; k < crewSims.length; k++) {
        const c = crewSims[k]
        const an = animators[k]
        const before = c.landed
        c.slot = rig.slots[k] * deckSx
        c.maxSway = rig.maxSway * Math.max(0.3, Math.abs(deckSx))
        c.step(boat, mirroredDeck, g, STEP, t)
        if (c.landed !== before && live) an.landed(t)
        if (an.hopDue) {
          an.hopDue = false
          if (live) c.hop(boat, mirroredDeck, Math.sqrt(2 * g * rig.crewH * 0.32))
        }
      }
      // Came about: somebody cheers (and maybe hops) as she picks up the new tack.
      if (boat.turns !== turnsSeen) {
        turnsSeen = boat.turns
        if (live && animators.length) {
          // Anyone but a lookout with the glass at its eye.
          const ok = animators.map((an, i) => (an.role === 'lookout' && !an.pixel ? -1 : i)).filter((i) => i >= 0)
          if (ok.length) {
            const pick = ok[Math.floor(hash(boat.turns * 131 + 7) * ok.length)]
            animators[pick].celebrate(t + 0.2)
            if (ok.length > 2 && hash(boat.turns * 17 + 3) < 0.5) animators[ok[(ok.indexOf(pick) + 2) % ok.length]].celebrate(t + 0.55)
          }
        }
      }
    }

    /** The pointer stirs the band whose water it is in: the frontmost surface above it. */
    const poke = (x: number, y: number, dv: number) => {
      for (let i = BANDS - 1; i >= 0; i--) {
        const b = bands[i]
        const sy = lutAt(b, x)
        if (sy > y + 10) continue
        const w = Math.exp(-Math.max(0, y - sy) / 70)
        if (w < 0.05) return
        const f = b.f
        const u = x / 8 + 3
        const i0 = Math.max(1, Math.floor(u - 4))
        const i1 = Math.min(f.length - 2, Math.ceil(u + 4))
        for (let j = i0; j <= i1; j++) {
          const d = (j - u) * 8
          b.v[j] += dv * w * Math.exp(-(d * d) / (14 * 14))
        }
        return
      }
    }

    const lutAt = (b: Band, x: number) => {
      const u = x / LUT_DX
      const i = clamp(Math.floor(u), 0, b.lut.length - 2)
      const f = clamp(u - i, 0, 1)
      return b.lut[i] + (b.lut[i + 1] - b.lut[i]) * f
    }

    // Pass 1: each band's trig, its row-0 particles, the surface lookup, foam.
    const prepBand = (b: Band, t: number) => {
      const off = b.drift * t
      const start = Math.floor(off / step)
      const frac = off - start * step
      const half = step / 2
      const { k, amp, omega, dir, phase, sn, cs, X, Y, S, J, foam } = b
      const steep = Math.max(0.05, b.steep)
      // Foam builds over ~250ms and fades over ~400ms (instantly on a still frame).
      const rise = frameDt > 0 ? frameDt * 4 : 1
      const fall = frameDt > 0 ? frameDt * 2.4 : 1
      for (let c = 0; c < cols; c++) {
        const x0 = c * step - frac + half
        let dx = 0
        let dy = 0
        let jx = 1
        let jy = 0
        for (let j = 0; j < NW; j++) {
          const th = dir[j] * k[j] * x0 - omega[j] * t + phase[j]
          const s = Math.sin(th)
          const co = Math.cos(th)
          sn[c * NW + j] = s
          cs[c * NW + j] = co
          const a = amp[j]
          dx -= dir[j] * a * s
          dy -= a * co
          jx -= k[j] * a * co
          jy += dir[j] * k[j] * a * s
        }
        const x = x0 + dx
        X[c] = x
        Y[c] = b.base + dy + springAt(b, x)
        S[c] = jy / Math.max(0.08, jx) + springSlope(b, x)
        J[c] = jx
        // Foam: where the crest is sharpest (particles crowd together), some columns whiten,
        // then the foam stays with that water and fades after the crest moves on.
        const col = start + c
        const fi = col & (FOAM_RING - 1)
        const crest = (1 - jx) / steep
        const spawn = hash(col * 31 + b.index * 977) < 0.5 ? clamp((crest - 0.66) / 0.22, 0, 1) : 0
        foam[fi] += clamp(spawn - foam[fi], -fall, rise)
      }
      // Surface lookup on a regular screen grid.
      const lut = b.lut
      let c = 0
      for (let i = 0; i < lut.length; i++) {
        const x = i * LUT_DX
        while (c < cols - 2 && X[c + 1] < x) c++
        const x1 = X[c]
        const x2 = X[c + 1]
        const f = x2 > x1 ? clamp((x - x1) / (x2 - x1), 0, 1) : 0
        lut[i] = Y[c] + (Y[c + 1] - Y[c]) * f
      }
    }

    const drawBand = (ctx: CanvasRenderingContext2D, b: Band, i: number, t: number) => {
      if (!atlas) return
      const off = b.drift * t
      const start = Math.floor(off / step)
      const { amp, dir, sn, cs, X, Y, S, J, foam, decay, rows } = b
      const lift = lineH * 0.62
      // Paper under the surface hides the bands behind this one.
      if (i > 0) {
        ctx.setTransform(dpr, 0, 0, dpr, 0, 0)
        ctx.globalAlpha = i === boatBand ? 0.87 : 1
        ctx.fillStyle = paper
        ctx.beginPath()
        ctx.moveTo(-step, H + 1)
        for (let c = 0; c < cols; c++) ctx.lineTo(X[c], Y[c] - lift)
        ctx.lineTo(W + step, H + 1)
        ctx.closePath()
        ctx.fill()
      }
      const alpha = Math.min(1, b.alpha / BODY_INK)
      ctx.globalAlpha = alpha
      const gw = cw / dpr
      const gh = ch / dpr
      const nRows = rows.length
      const next = i < BANDS - 1 ? bands[i + 1] : null
      const half = step / 2
      const frac = off - start * step
      for (let c = 0; c < cols; c++) {
        const xc = X[c]
        if (xc < -step * 2 || xc > W + step * 2) continue
        const floor = next ? lutAt(next, xc) + lineH * 0.45 : H + lineH
        const x0 = c * step - frac + half
        const col = start + c
        const fo = foam[col & (FOAM_RING - 1)]
        const hs = springAt(b, xc)
        const yc = Y[c]
        // One transform per column: the surface's lean and crest compression. Every row of the
        // column is placed inside it at its own Gerstner position (deeper rows orbit less).
        const sx = 1 - (1 - J[c]) * 0.3
        const lean = clamp(S[c], -0.9, 0.9)
        ctx.setTransform(sx * dpr, sx * lean * dpr, 0, dpr, xc * dpr, yc * dpr)
        const isx = 1 / sx
        for (let r = 0; r < nRows; r++) {
          let u = 0
          let v = 0
          let y = yc
          if (r > 0) {
            let dx = 0
            let dy = 0
            const dr = r * NW
            const cb = c * NW
            for (let j = 0; j < NW; j++) {
              const a = amp[j] * decay[dr + j]
              dx -= dir[j] * a * sn[cb + j]
              dy -= a * cs[cb + j]
            }
            const x = x0 + dx
            y = b.base + r * lineH + dy + hs * Math.exp(-r * 0.6)
            u = (x - xc) * isx
            v = y - yc - lean * (x - xc)
          }
          if (y - lineH * 0.5 > floor || y - lineH > H) break
          const text = rows[r]
          const code = text[(start + c + r * 7) % text.length]
          const g = code & 127
          const foamy = r === 0 && fo > 0.02
          if (g === 0 && !foamy) continue
          if (foamy) {
            const fg = FOAM_IDX[Math.floor(hash(col * 7 + i) * FOAM_IDX.length)]
            ctx.globalAlpha = alpha * fo
            ctx.drawImage(atlas, fg * cw, SET_CREST * ch, cw, ch, u - gw / 2, v - gh / 2 - lineH * 0.12, gw, gh)
            if (g !== 0 && fo < 0.98) {
              ctx.globalAlpha = alpha * (1 - fo)
              const set = code & ACCENT ? SET_ACCENT : SET_CREST
              ctx.drawImage(atlas, g * cw, set * ch, cw, ch, u - gw / 2, v - gh / 2, gw, gh)
            }
            ctx.globalAlpha = alpha
            continue
          }
          const set = code & ACCENT ? SET_ACCENT : r === 0 ? SET_CREST : SET_BODY
          ctx.drawImage(atlas, g * cw, set * ch, cw, ch, u - gw / 2, v - gh / 2, gw, gh)
        }
      }
      ctx.globalAlpha = 1
    }

    // The painted boat boils slowly: each frame held ~300ms, crossfading into the next.
    const BOIL_HOLD = 0.3
    const BOIL_XF = 0.1

    const drawCrew = (t: number, alphaI: number) => {
      cctx.setTransform(1, 0, 0, 1, 0, 0)
      cctx.clearRect(0, 0, crewCv.width, crewCv.height)
      if (!boat || !rig || boatBand < 0) return
      const x = lerp(boat.px, boat.x, alphaI)
      const y = lerp(boat.py, boat.y, alphaI)
      const a = lerp(boat.pa, boat.a, alphaI)
      const sx = lerp(boat.psx, boat.sx, alphaI)
      const [ox, oy] = boat.pivot(x, y, a, sx)
      const co = Math.cos(a)
      const si = Math.sin(a)
      const man = assets?.manifest
      cctx.imageSmoothingEnabled = true
      cctx.imageSmoothingQuality = 'high'
      if (rig.painted && boatSheet && man?.boat) {
        const nF = man.boat.frames
        const u = t / BOIL_HOLD
        const fi = ((Math.floor(u) % nF) + nF) % nF
        const into = (u - Math.floor(u)) * BOIL_HOLD
        const blend = clamp((into - (BOIL_HOLD - BOIL_XF)) / BOIL_XF, 0, 1)
        cctx.setTransform(co * sx * dpr, si * sx * dpr, -si * dpr, co * dpr, ox * dpr, oy * dpr)
        cctx.drawImage(boatSheet, fi * boatCell.w, 0, boatCell.w, boatCell.h, -rig.ox, -rig.oy, rig.w, rig.h)
        if (blend > 0) {
          cctx.globalAlpha = blend * blend * (3 - 2 * blend)
          cctx.drawImage(boatSheet, ((fi + 1) % nF) * boatCell.w, 0, boatCell.w, boatCell.h, -rig.ox, -rig.oy, rig.w, rig.h)
          cctx.globalAlpha = 1
        }
      } else {
        const tf = `translate3d(${(ox - rig.ox).toFixed(2)}px, ${(oy - rig.oy).toFixed(2)}px, 0) rotate(${a.toFixed(4)}rad) scaleX(${sx.toFixed(4)})`
        boatEl.style.transform = tf
        railEl.style.transform = tf
      }
      const recent = t - ptr.sim < 2.5
      const facing = sx < 0 || (sx === 0 && boat.dir < 0) ? -1 : 1
      // Crew turn with the boat: they narrow towards side-on and come out mirrored.
      const squeeze = facing * (0.45 + 0.55 * Math.abs(sx))
      for (let k = 0; k < crewSims.length; k++) {
        const c = crewSims[k]
        const an = animators[k]
        const fx = lerp(c.pfx, c.fx, alphaI)
        const fy = lerp(c.pfy, c.fy, alphaI)
        if (rig.crewPainted && crewSheets[k] && man?.crew) {
          const m = man.crew.members[k]
          const def = m.anims[an.anim] ?? m.anims.idle
          const sc = rig.crewScale
          // Lean against the boat's acceleration and with their own sway.
          const lean = clamp(a * 0.4 - boat.ax * 0.0035 + c.sv * 0.004, -0.16, 0.16)
          const cl = Math.cos(lean)
          const sl = Math.sin(lean)
          cctx.setTransform(cl * squeeze * dpr, sl * squeeze * dpr, -sl * dpr, cl * dpr, fx * dpr, fy * dpr)
          const dx = -man.crew.anchor.x * sc
          const dy = -man.crew.anchor.y * sc
          const dw = man.crew.frame.w * sc
          const dh = man.crew.frame.h * sc
          const cell = (f: number, row: number) => crewSheets[k] && cctx.drawImage(crewSheets[k], f * crewCell.w, row * crewCell.h, crewCell.w, crewCell.h, dx, dy, dw, dh)
          if (an.anim === 'idle' || (an.anim === 'wave' && an.end === Infinity)) {
            // Loops boil gently: idle at 8 fps crossfading frame to frame, wave at 10 fps.
            const fps = an.anim === 'idle' ? 8 : 10
            const u = (t - an.start) * fps
            const f0 = ((Math.floor(u) % def.frames) + def.frames) % def.frames
            cell(f0, def.row)
            const blend = an.anim === 'idle' ? clamp((u - Math.floor(u) - 0.55) / 0.45, 0, 1) : 0
            if (blend > 0) {
              cctx.globalAlpha = blend
              cell((f0 + 1) % def.frames, def.row)
              cctx.globalAlpha = 1
            }
          } else {
            cell(an.frame(t, def.frames, def.loop), def.row)
          }
        } else if (pal) {
          const look = recent && Math.abs(ptr.x - fx) > 30 && Math.abs(ptr.x - fx) < 420 && Math.abs(ptr.y - fy) < 300 ? Math.sign(ptr.x - fx) : 0
          const pose = an.pose(kinds[k], t, c.air, c.hvy < 0, look * facing)
          const img = clawdFrame(kinds[k], pose, { ...pal, body: bodyColors[k % bodyColors.length] }, rig.cell)
          if (!img) continue
          // Pixel art stays on the device-pixel grid; it mirrors when the boat comes about.
          const px = Math.round(fx * dpr)
          const py = Math.round(fy * dpr - (FEET.y - BOX.y) * rig.cell)
          const feetX = (FEET.x - BOX.x) * rig.cell
          if (facing < 0) cctx.setTransform(-1, 0, 0, 1, px + feetX, 0)
          else cctx.setTransform(1, 0, 0, 1, px - feetX, 0)
          cctx.drawImage(img, 0, py)
        }
      }
      cctx.setTransform(1, 0, 0, 1, 0, 0)
    }

    // Dev builds report the draw cost on the canvas (data-frame-ms="avg/max" over ~2s).
    let statN = 0
    let statSum = 0
    let statMax = 0
    // physics, prep, sea draw, crew draw, frames
    const prof = [0, 0, 0, 0, 0]

    const frame = (dt: number, live: boolean) => {
      if (!atlas) return
      const t0 = import.meta.env.DEV ? performance.now() : 0
      frameDt = dt
      if (live) {
        acc += dt
        let n = 0
        while (acc >= STEP && n < 24) {
          simT += STEP
          physicsStep(simT, true)
          acc -= STEP
          n++
        }
        if (n === 24) acc = 0
        const t = simT
        for (let k = 0; k < animators.length; k++) {
          const an = animators[k]
          an.update(t, crewSims[k]?.air ?? false)
        }
        // Greet a pointer that comes near the boat: the nearest crew member waves.
        if (boat && t - ptr.sim < 0.3 && t - lastGreet > 3.5 && crewSims.length) {
          let best = -1
          let bd = 150
          for (let k = 0; k < crewSims.length; k++) {
            const d = Math.hypot(crewSims[k].fx - ptr.x, crewSims[k].fy - ptr.y)
            if (d < bd) {
              bd = d
              best = k
            }
          }
          if (best >= 0) {
            animators[best].greet(t)
            lastGreet = t
          }
        }
      }
      const alphaI = live ? acc / STEP : 1
      const tw = simT - STEP + alphaI * STEP
      const t1 = import.meta.env.DEV ? performance.now() : 0
      for (const b of bands) prepBand(b, tw)
      const t2 = import.meta.env.DEV ? performance.now() : 0
      // The boat's own band is drawn in front of it: the hull sits in the water, cut at the
      // waterline by the waves, its submerged part faintly visible through them.
      const split = boatBand < 0 ? BANDS : boatBand
      bctx.setTransform(1, 0, 0, 1, 0, 0)
      bctx.clearRect(0, 0, back.width, back.height)
      for (let i = 0; i < split; i++) drawBand(bctx, bands[i], i, tw)
      fctx.setTransform(1, 0, 0, 1, 0, 0)
      fctx.clearRect(0, 0, front.width, front.height)
      for (let i = split; i < BANDS; i++) drawBand(fctx, bands[i], i, tw)
      const t3 = import.meta.env.DEV ? performance.now() : 0
      drawCrew(tw, alphaI)
      if (import.meta.env.DEV) {
        const t4 = performance.now()
        prof[0] += t1 - t0
        prof[1] += t2 - t1
        prof[2] += t3 - t2
        prof[3] += t4 - t3
        prof[4]++
      }
      if (import.meta.env.DEV) {
        ;(window as unknown as { __sea?: unknown }).__sea = { boat, course, rig, boatBand, bands, crewSims, animators, simT, prof, dpr }
        const ms = performance.now() - t0
        statSum += ms
        statMax = Math.max(statMax, ms)
        if (++statN >= 120) {
          back.dataset.frameMs = `${(statSum / statN).toFixed(2)}/${statMax.toFixed(2)}`
          statN = statSum = statMax = 0
        }
      }
    }

    let raf = 0
    let last = 0
    let running = false
    let ready = false
    let disposed = false

    const loop = (now: number) => {
      raf = requestAnimationFrame(loop)
      const dt = last ? Math.min(0.1, (now - last) / 1000) : 0
      last = now
      frame(dt, true)
    }
    const play = () => {
      if (!ready || running || reduce.matches || document.hidden) return
      running = true
      last = 0
      raf = requestAnimationFrame(loop)
    }
    const stop = () => {
      running = false
      cancelAnimationFrame(raf)
    }
    const refresh = () => {
      if (!ready) return
      layout()
      const show = !!rig && boatBand >= 0
      for (const el of [boatEl, railEl, crewCv]) el.classList.toggle('ready', show)
      if (reduce.matches) {
        stop()
        frame(0, false)
      } else {
        frame(0, false)
        play()
      }
    }

    let pending = 0
    const schedule = () => {
      cancelAnimationFrame(pending)
      pending = requestAnimationFrame(refresh)
    }
    const onVisibility = () => (document.hidden ? stop() : play())

    // Pointer: hover on desktop, touch on phones. Moving across the water drags ripples.
    const onMove = (cx: number, cy: number) => {
      if (!running) return
      const r = back.getBoundingClientRect()
      const x = cx - r.left
      const y = cy - r.top
      const now = performance.now()
      const dtp = Math.max(0.008, (now - ptr.t) / 1000)
      if (ptr.t && dtp < 0.25) {
        const v = Math.min(1600, Math.hypot(x - ptr.x, y - ptr.y) / dtp)
        if (v > 30 && ptr.queued.length < 16) ptr.queued.push({ x, y, dv: v * 0.028 })
      }
      ptr.x = x
      ptr.y = y
      ptr.t = now
      ptr.sim = simT
    }
    const onPointer = (e: PointerEvent) => onMove(e.clientX, e.clientY)
    const onTouch = (e: TouchEvent) => {
      const tch = e.touches[0]
      if (tch) onMove(tch.clientX, tch.clientY)
    }

    const ro = new ResizeObserver(schedule)
    ro.observe(back)
    if (sky.current) ro.observe(sky.current)
    const mo = new MutationObserver(refresh)
    mo.observe(root, { attributes: true, attributeFilter: ['data-theme'] })
    document.addEventListener('visibilitychange', onVisibility)
    reduce.addEventListener('change', refresh)
    scheme.addEventListener('change', refresh)
    window.addEventListener('pointermove', onPointer, { passive: true })
    window.addEventListener('touchmove', onTouch, { passive: true })
    window.addEventListener('touchstart', onTouch, { passive: true })

    // Painted art, if it's there; otherwise (or after a short wait) the fallbacks.
    const settle = (a: Assets | null) => {
      if (disposed) return
      if (a && a !== assets) {
        assets = a
        const mb = a.manifest.boat
        if (a.boat && mb) {
          boatArt = scanAlpha(a.boat, 0, 0, mb.frame.w, mb.frame.h)
          boatRows = scanRows(a.boat, mb.frame.w, mb.frame.h)
        }
        const mc = a.manifest.crew
        if (a.crew && mc) {
          // Widest idle pose across the crew (arms out), for spacing.
          crewArtW = 0
          a.crew.forEach((img, i) => {
            const row = mc.members[i].anims.idle.row
            const bb = scanAlpha(img, 0, row * mc.frame.h, mc.frame.w, mc.frame.h)
            crewArtW = Math.max(crewArtW, bb.right - bb.left)
          })
        }
        if (assetsSettled) schedule()
      }
      if (!assetsSettled) {
        assetsSettled = true
        schedule()
      }
    }
    // Dev only: ?fallback previews the pixel crew and SVG boat.
    const forceFallback = import.meta.env.DEV && new URLSearchParams(location.search).has('fallback')
    const assetLoad = forceFallback ? Promise.resolve(null) : loadAssets()
    void assetLoad.then(settle)
    const waitAssets = Promise.race([assetLoad, new Promise<null>((r) => setTimeout(() => r(null), 3000))])

    const fonts = document.fonts
    const loaded = fonts ? fonts.load(`400 13px ${FAMILY}`, SNIPPETS.slice(0, 20).join('')) : Promise.resolve()
    void Promise.race([loaded, new Promise((r) => setTimeout(r, 2500))])
      .catch(() => {})
      .then(() => {
        if (disposed) return
        ready = true
        refresh()
        void waitAssets.then((a) => settle(a))
      })

    return () => {
      disposed = true
      stop()
      cancelAnimationFrame(pending)
      ro.disconnect()
      mo.disconnect()
      document.removeEventListener('visibilitychange', onVisibility)
      reduce.removeEventListener('change', refresh)
      scheme.removeEventListener('change', refresh)
      window.removeEventListener('pointermove', onPointer)
      window.removeEventListener('touchmove', onTouch)
      window.removeEventListener('touchstart', onTouch)
    }
  }, [sky])

  return (
    <>
      <canvas ref={backRef} className="sea" aria-hidden="true" />
      <div ref={boatRef} className="sea-boat" aria-hidden="true">
        <Boat />
      </div>
      <canvas ref={crewRef} className="sea sea-crew" aria-hidden="true" />
      <div ref={railRef} className="sea-boat" aria-hidden="true">
        <BoatRail />
      </div>
      <canvas ref={frontRef} className="sea sea-front" aria-hidden="true" />
    </>
  )
}
