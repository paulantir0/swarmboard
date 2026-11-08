import { useEffect, useRef, type RefObject } from 'react'
import { BOAT_H, BOAT_TOP, BOAT_W, Boat, DECK_X0, DECK_X1, WATERLINE, deckY } from './Boat'
import { CREW, CREW_CX, CREW_FEET, CREW_PITCH, CREW_VIEW, Critter } from './Crew'
import { SNIPPETS, isMarker } from './seaText'

// A side-view sea of code-review noise, like a 2D illustration: four stacked bands, each a
// rolling wave line (two sines, wavelengths ~300-600px, several rows tall) with rows of PR
// chatter hanging under it and following its contour. Each band scrolls at its own speed
// (parallax), back bands lighter, front bands darker, and each one paints paper under its line
// so it hides the bands behind. The boat rides the crest line of a middle band; the bands in
// front are drawn on a second canvas above it, so they wash across the hull.
//
// Cost: one glyph atlas (ink + accent), rows as typed arrays of glyph indices, the wave line
// computed once per column per band, 24fps, paused when hidden, one still frame for reduced motion.

const FPS = 24
const FAMILY = "'Swarm Mono', ui-monospace, monospace"
const TAU = Math.PI * 2
const BANDS = 4

const lerp = (a: number, b: number, t: number) => a + (b - a) * t

// Glyph table: every character used by the snippets. Index < 128, so the high bit marks accent.
const CHARS = [...new Set([' ', ...SNIPPETS.join('')])]
const INDEX = new Map(CHARS.map((c, i) => [c, i]))
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

interface Band {
  base: number
  amp: number
  k1: number
  k2: number
  ph1: number
  ph2: number
  speed: number // wave travel, px/s (leftwards)
  textSpeed: number
  alpha: number
  rows: Uint8Array[]
}

export function Sea({ sky }: { sky: RefObject<HTMLElement | null> }) {
  const backRef = useRef<HTMLCanvasElement>(null)
  const frontRef = useRef<HTMLCanvasElement>(null)
  const boatRef = useRef<HTMLDivElement>(null)
  const crewRef = useRef<(HTMLDivElement | null)[]>([])

  useEffect(() => {
    const back = backRef.current
    const front = frontRef.current
    const boat = boatRef.current
    const crew = crewRef.current
    if (!back || !front || !boat) return
    const bctx = back.getContext('2d')
    const fctx = front.getContext('2d')
    if (!bctx || !fctx) return

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
    let cw = 8 // device px per glyph cell
    let ch = 16
    let paper = '#ffffff'
    let cols = 0
    let wy = new Float32Array(0)

    // Boat and crew
    let boatBand = -1
    let bw = 0
    let bs = 1
    let bxMid = 0
    let bxAmp = 0
    let px = 3 // crew pixel size, whole CSS px

    const buildAtlas = (size: number) => {
      const css = getComputedStyle(root)
      const ink = css.getPropertyValue('--sea-ink').trim() || '17, 17, 17'
      const accent = css.getPropertyValue('--sea-accent').trim() || '44, 74, 124'
      paper = css.getPropertyValue('--color-canvas').trim() || '#ffffff'
      cw = Math.max(2, Math.round(size * 0.6 * dpr))
      ch = Math.ceil(size * 1.4 * dpr)
      const img = document.createElement('canvas')
      img.width = cw * CHARS.length
      img.height = ch * 2
      const g = img.getContext('2d')
      if (!g) return null
      g.font = `400 ${size * dpr}px ${FAMILY}`
      g.textAlign = 'center'
      g.textBaseline = 'middle'
      for (let set = 0; set < 2; set++) {
        g.fillStyle = `rgb(${set ? accent : ink})`
        for (let i = 1; i < CHARS.length; i++) g.fillText(CHARS[i], i * cw + cw / 2, set * ch + ch / 2)
      }
      return img
    }

    const line = (b: Band, x: number, t: number) =>
      b.base + b.amp * (0.78 * Math.sin(b.k1 * (x + b.speed * t) + b.ph1) + 0.22 * Math.sin(b.k2 * (x + b.speed * 1.6 * t) + b.ph2))

    const layout = () => {
      const rect = back.getBoundingClientRect()
      W = Math.max(1, Math.round(rect.width))
      H = Math.max(1, Math.round(rect.height))
      dpr = Math.min(2, window.devicePixelRatio || 1)
      for (const c of [back, front]) {
        c.width = Math.ceil(W * dpr)
        c.height = Math.ceil(H * dpr)
      }
      const small = W < 640
      const el = sky.current
      const skyBottom = el ? el.offsetTop + el.offsetHeight : H * 0.4
      const skyRight = el ? el.offsetLeft + el.offsetWidth : W * 0.5
      const size = small ? 11 : 13
      lineH = Math.round(size * 1.38)
      atlas = buildAtlas(size)
      const step = cw / dpr
      cols = Math.ceil(W / step) + 2
      wy = new Float32Array(cols)

      // Bands: the back crest can reach seaTop, the rest stack evenly to the bottom edge.
      const seaTop = Math.min(H - 120, Math.max(H * 0.45, skyBottom + 10))
      const A = small ? 22 : Math.min(52, Math.max(28, (H - seaTop) * 0.12))
      const base0 = seaTop + A
      const gap = (H - base0) / (BANDS - 0.15)
      const dark = isDark()
      const rand = mulberry32(0x5eab0a7)
      bands = []
      for (let i = 0; i < BANDS; i++) {
        const z = i / (BANDS - 1)
        const amp = A * lerp(0.8, 1.05, z)
        const lambda = lerp(640, 440, z) * (small ? 0.8 : 1)
        bands.push({
          base: base0 + i * gap,
          amp,
          k1: TAU / lambda,
          k2: TAU / (lambda * 0.58),
          ph1: i * 1.9,
          ph2: i * 0.7 + 1,
          speed: lerp(12, 40, z) * (small ? 0.7 : 1),
          textSpeed: lerp(7, 22, z) * (small ? 0.7 : 1),
          alpha: lerp(0.15, dark ? 0.42 : 0.5, z),
          rows: [],
        })
      }
      for (let i = 0; i < BANDS; i++) {
        const b = bands[i]
        // Rows reach just past the lowest trough of the band in front (anything lower is hidden).
        const bottom = i < BANDS - 1 ? bands[i + 1].base + bands[i + 1].amp + lineH : H + lineH
        const n = Math.max(1, Math.ceil((bottom - (b.base - b.amp)) / lineH) + 1)
        for (let r = 0; r < n; r++) b.rows.push(rowText(rand, cols * 2 + 60))
      }

      // Boat: long enough that six crew at a readable pixel size fit on the deck. On short
      // screens it shrinks until its masthead clears the copy; if it never does, it stays ashore.
      const fit = (width: number) => {
        bw = width
        bs = bw / BOAT_W
        const above = (WATERLINE - BOAT_TOP) * bs
        const free = W - 8 - (skyRight + 16) - bw
        let overlaps = true
        if (!small && free >= 0) {
          bxAmp = Math.min(W * 0.05, free / 2)
          bxMid = skyRight + 16 + bw / 2 + free / 2
          overlaps = false
        } else {
          bxMid = W / 2 + (small ? 0 : W * 0.12)
          bxAmp = small ? Math.max(0, (W - bw) / 2 - 4) : W * 0.04
        }
        for (let i = 1; i < BANDS; i++) {
          const b = bands[i]
          if (overlaps && b.base - b.amp - above < skyBottom + 6) continue
          // The last band only as a fallback: it has nothing in front to wash over the hull.
          if (i === BANDS - 1 && width > minBw) return -1
          return i
        }
        return -1
      }
      const fullBw = small ? Math.min(W - 24, 360) : Math.min(580, Math.max(300, W * 0.4))
      const minBw = Math.min(fullBw, small ? W * 0.62 : 260)
      boatBand = -1
      for (let width = fullBw; boatBand < 0 && width >= minBw - 0.5; width = width > minBw ? Math.max(minBw, width * 0.92) : minBw - 1) {
        boatBand = fit(width)
      }
      px = Math.max(2, Math.min(6, Math.floor(((DECK_X1 - DECK_X0) * bs) / (CREW_PITCH * CREW.length + 2))))
      boat.style.width = `${bw}px`
      boat.style.height = `${BOAT_H * bs}px`
      boat.style.transformOrigin = `${bw / 2}px ${WATERLINE * bs}px`
      boat.style.display = boatBand < 0 ? 'none' : ''
      for (const c of crew) {
        if (!c) continue
        c.style.width = `${CREW_VIEW.w * px}px`
        c.style.height = `${CREW_VIEW.h * px}px`
        c.style.display = boatBand < 0 ? 'none' : ''
      }
    }

    const drawBand = (ctx: CanvasRenderingContext2D, b: Band, i: number, t: number) => {
      if (!atlas) return
      const step = cw / dpr
      const off = b.textSpeed * t
      const start = Math.floor(off / step)
      const frac = off - start * step
      for (let c = 0; c < cols; c++) wy[c] = line(b, c * step - frac + step / 2, t)
      // Paper under the wave line hides the bands behind this one.
      if (i > 0) {
        ctx.globalAlpha = 1
        ctx.fillStyle = paper
        ctx.beginPath()
        ctx.moveTo(0, H * dpr)
        const lift = lineH * 0.62
        for (let c = 0; c < cols; c++) ctx.lineTo((c * step - frac) * dpr, (wy[c] - lift) * dpr)
        ctx.lineTo(W * dpr, H * dpr)
        ctx.closePath()
        ctx.fill()
      }
      const half = ch / 2
      for (let r = 0; r < b.rows.length; r++) {
        const text = b.rows[r]
        const len = text.length
        // The crest row is a touch darker so each wave line reads as a shape.
        ctx.globalAlpha = r === 0 ? Math.min(1, b.alpha * 1.35) : b.alpha * (r === 1 ? 1 : 0.9)
        const dy = r * lineH
        for (let c = 0; c < cols; c++) {
          const code = text[(start + c + r * 7) % len]
          const g = code & 127
          if (g === 0) continue
          const y = (wy[c] + dy) * dpr - half
          if (y > H * dpr) continue
          ctx.drawImage(atlas, g * cw, code >> 7 ? ch : 0, cw, ch, Math.round((c * step - frac) * dpr), Math.round(y), cw, ch)
        }
      }
      ctx.globalAlpha = 1
    }

    const placeBoat = (t: number) => {
      if (boatBand < 0) return
      const b = bands[boatBand]
      const x = bxMid + bxAmp * Math.sin(t * 0.04 + 1.1)
      const h = line(b, x, t)
      const span = bw * 0.3
      const slope = (line(b, x + span, t) - line(b, x - span, t)) / (2 * span)
      const roll = Math.max(-0.3, Math.min(0.3, Math.atan(slope) * 0.85))
      const tx = x - bw / 2
      const ty = h - WATERLINE * bs
      boat.style.transform = `translate3d(${tx.toFixed(1)}px, ${ty.toFixed(1)}px, 0) rotate(${roll.toFixed(4)}rad)`
      // Crew: stand on the rolled deck, kept upright and snapped to whole pixels.
      const ox = bw / 2
      const oy = WATERLINE * bs
      const cs = Math.cos(roll)
      const sn = Math.sin(roll)
      const mid = (DECK_X0 + DECK_X1) / 2
      for (let k = 0; k < crew.length; k++) {
        const el = crew[k]
        if (!el) continue
        const lx = mid + ((k - (CREW.length - 1) / 2) * CREW_PITCH * px) / bs
        const ly = deckY(lx)
        const dx = lx * bs - ox
        const dy = ly * bs - oy
        const fx = tx + ox + dx * cs - dy * sn
        const fy = ty + oy + dx * sn + dy * cs
        const left = Math.round(fx - (CREW_CX - CREW_VIEW.x) * px)
        const top = Math.round(fy - (CREW_FEET - CREW_VIEW.y) * px)
        el.style.transform = `translate3d(${left}px, ${top}px, 0)`
      }
    }

    const draw = (t: number) => {
      if (!atlas) return
      const split = boatBand < 0 ? BANDS : boatBand + 1
      for (const [ctx, from, to] of [
        [bctx, 0, split],
        [fctx, split, BANDS],
      ] as const) {
        ctx.setTransform(1, 0, 0, 1, 0, 0)
        ctx.clearRect(0, 0, back.width, back.height)
        for (let i = from; i < to; i++) drawBand(ctx, bands[i], i, t)
      }
      placeBoat(t)
    }

    let raf = 0
    let last = 0
    let running = false
    let ready = false
    let disposed = false
    const t0 = performance.now()
    const STILL_T = 7

    const loop = (now: number) => {
      raf = requestAnimationFrame(loop)
      if (now - last < 1000 / FPS - 2) return
      last = now
      draw((now - t0) / 1000)
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
      boat.classList.add('ready')
      for (const c of crew) c?.classList.add('ready')
      if (reduce.matches) {
        stop()
        draw(STILL_T)
      } else {
        draw((performance.now() - t0) / 1000)
        play()
      }
    }

    let pending = 0
    const schedule = () => {
      cancelAnimationFrame(pending)
      pending = requestAnimationFrame(refresh)
    }
    const onVisibility = () => (document.hidden ? stop() : play())

    const ro = new ResizeObserver(schedule)
    ro.observe(back)
    if (sky.current) ro.observe(sky.current)
    const mo = new MutationObserver(refresh)
    mo.observe(root, { attributes: true, attributeFilter: ['data-theme'] })
    document.addEventListener('visibilitychange', onVisibility)
    reduce.addEventListener('change', refresh)
    scheme.addEventListener('change', refresh)

    const fonts = document.fonts
    const loaded = fonts ? fonts.load(`400 13px ${FAMILY}`, SNIPPETS.slice(0, 20).join('')) : Promise.resolve()
    void Promise.race([loaded, new Promise((r) => setTimeout(r, 2500))])
      .catch(() => {})
      .then(() => {
        if (disposed) return
        ready = true
        refresh()
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
    }
  }, [sky])

  return (
    <>
      <canvas ref={backRef} className="sea" aria-hidden="true" />
      <div ref={boatRef} className="sea-boat" aria-hidden="true">
        <Boat />
      </div>
      {CREW.map((c, i) => (
        <div
          key={c.kind}
          ref={(el) => {
            crewRef.current[i] = el
          }}
          className="sea-crew"
          aria-hidden="true"
        >
          <Critter kind={c.kind} color={c.color} delay={i * 0.61} />
        </div>
      ))}
      <canvas ref={frontRef} className="sea sea-front" aria-hidden="true" />
    </>
  )
}
