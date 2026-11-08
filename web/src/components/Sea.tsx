import { useEffect, useRef, type RefObject } from 'react'
import { BOAT_H, BOAT_TOP, BOAT_W, Boat, BoatRail, DECK_MID, DECK_X0, DECK_X1, WATERLINE, deckY } from './Boat'
import { CREW, CREW_CX, CREW_FEET, CREW_PITCH, CREW_VIEW, Critter } from './Crew'
import { SNIPPETS, isMarker } from './seaText'

// A side-view sea of code-review noise: four stacked bands, each a long rolling wave line with rows
// of PR chatter hanging under it. Glyphs sit at sub-pixel positions and lean with the local slope
// (a per-column shear), so the text flows over crests instead of stair-stepping. Each band scrolls at its own speed (back
// lighter and slower, front darker and faster) and paints paper under its line to hide the bands
// behind. The boat rides the crest of a middle band, its height and roll eased by critically damped
// springs; the bands in front are on a second canvas, so they wash across the hull.
//
// Cost per frame: one transform per column per band (all rows of a column share it), one
// drawImage per visible glyph from a pre-baked atlas, reused typed arrays, no allocation.
// Every animation frame, all motion from elapsed time; paused when hidden; still under reduced motion.

const FAMILY = "'Swarm Mono', ui-monospace, monospace"
const TAU = Math.PI * 2
const BANDS = 4
// Atlas rows: body text, crest text (fuller ink), merge markers (accent).
const SET_BODY = 0
const SET_CREST = 1
const SET_ACCENT = 2
const BODY_INK = 0.74

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
  k3: number
  ph1: number
  ph2: number
  ph3: number
  speed: number // wave travel, px/s (leftwards)
  textSpeed: number
  alpha: number
  rows: Uint8Array[]
}

// Two long swells and one gentle ripple; height and its slope.
const height = (b: Band, x: number, t: number) =>
  b.base +
  b.amp *
    (0.74 * Math.sin(b.k1 * (x + b.speed * t) + b.ph1) +
      0.21 * Math.sin(b.k2 * (x + b.speed * 1.4 * t) + b.ph2) +
      0.05 * Math.sin(b.k3 * (x - b.speed * 0.6 * t) + b.ph3))
const slope = (b: Band, x: number, t: number) =>
  b.amp *
  (0.74 * b.k1 * Math.cos(b.k1 * (x + b.speed * t) + b.ph1) +
    0.21 * b.k2 * Math.cos(b.k2 * (x + b.speed * 1.4 * t) + b.ph2) +
    0.05 * b.k3 * Math.cos(b.k3 * (x - b.speed * 0.6 * t) + b.ph3))

/** Critically damped spring step toward `target`. */
function spring(pos: number, vel: number, target: number, omega: number, dt: number): [number, number] {
  const acc = omega * omega * (target - pos) - 2 * omega * vel
  vel += acc * dt
  return [pos + vel * dt, vel]
}

export function Sea({ sky }: { sky: RefObject<HTMLElement | null> }) {
  const backRef = useRef<HTMLCanvasElement>(null)
  const frontRef = useRef<HTMLCanvasElement>(null)
  const boatRef = useRef<HTMLDivElement>(null)
  const railRef = useRef<HTMLDivElement>(null)
  const crewRef = useRef<(HTMLDivElement | null)[]>([])

  useEffect(() => {
    const back = backRef.current
    const front = frontRef.current
    const boat = boatRef.current
    const rail = railRef.current
    const crew = crewRef.current
    if (!back || !front || !boat || !rail) return
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
    let cw = 8 // atlas cell, device px
    let ch = 16
    let step = 8 // glyph advance, CSS px
    let paper = '#ffffff'
    let cols = 0
    let wy = new Float32Array(0)
    let ws = new Float32Array(0)

    // Boat, crew and their eased pose
    let boatBand = -1
    let bw = 0
    let bs = 1
    let bxMid = 0
    let bxAmp = 0
    let cell = 3 // crew cell width, CSS px (height is twice that)
    let poseY = Number.NaN
    let poseVY = 0
    let poseR = 0
    let poseVR = 0

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
      cols = Math.ceil(W / step) + 3
      wy = new Float32Array(cols)
      ws = new Float32Array(cols)

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
        const lambda = lerp(920, 700, z) * (small ? 0.72 : 1)
        bands.push({
          base: base0 + i * gap,
          amp: A * lerp(0.8, 1.05, z),
          k1: TAU / lambda,
          k2: TAU / (lambda * 0.57),
          k3: TAU / (lambda * 0.31),
          ph1: i * 1.9,
          ph2: i * 0.7 + 1,
          ph3: i * 2.3 + 0.4,
          speed: lerp(9, 24, z) * (small ? 0.75 : 1),
          textSpeed: lerp(5, 14, z) * (small ? 0.75 : 1),
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

      // Boat: sized so six crew fit on the deck at a readable cell size. If its masthead can't
      // clear the copy, the crew (and boat) step down a size; failing that, it stays ashore.
      const fit = (cellW: number) => {
        const deckPx = (CREW_PITCH * CREW.length + 8) * cellW
        bs = deckPx / (DECK_X1 - DECK_X0)
        bw = BOAT_W * bs
        if (bw > W - 8) return -1
        const above = (WATERLINE - BOAT_TOP) * bs
        const free = W - 8 - (skyRight + 16) - bw
        let overlaps = true
        if (!small && free >= 0) {
          bxAmp = Math.min(W * 0.05, free / 2)
          bxMid = skyRight + 16 + bw / 2 + free / 2
          overlaps = false
        } else {
          bxMid = W / 2 + (small ? 0 : W * 0.12)
          bxAmp = Math.max(0, Math.min(small ? 24 : W * 0.04, (W - bw) / 2 - 4))
        }
        for (let i = 1; i < BANDS; i++) {
          const b = bands[i]
          if (overlaps && b.base - b.amp - above < skyBottom + 6) continue
          // The last band only as a fallback: nothing in front of it washes over the hull.
          if (i === BANDS - 1 && cellW > 2) return -1
          return i
        }
        return -1
      }
      boatBand = -1
      for (cell = small ? 2 : 3; cell >= 2 && boatBand < 0; cell--) boatBand = fit(cell)
      cell++
      poseY = Number.NaN

      for (const layer of [boat, rail]) {
        layer.style.width = `${bw}px`
        layer.style.height = `${BOAT_H * bs}px`
        layer.style.transformOrigin = `${bw / 2}px ${WATERLINE * bs}px`
        layer.style.display = boatBand < 0 ? 'none' : ''
      }
      for (const c of crew) {
        if (!c) continue
        c.style.width = `${CREW_VIEW.w * cell}px`
        c.style.height = `${CREW_VIEW.h * cell * 2}px`
        c.style.display = boatBand < 0 ? 'none' : ''
      }
    }

    const drawBand = (ctx: CanvasRenderingContext2D, b: Band, i: number, t: number) => {
      if (!atlas) return
      const off = b.textSpeed * t
      const start = Math.floor(off / step)
      const frac = off - start * step
      const half = step / 2
      for (let c = 0; c < cols; c++) {
        const x = c * step - frac + half
        wy[c] = height(b, x, t)
        ws[c] = slope(b, x, t)
      }
      // Paper under the wave line hides the bands behind this one.
      if (i > 0) {
        ctx.setTransform(dpr, 0, 0, dpr, 0, 0)
        ctx.globalAlpha = 1
        ctx.fillStyle = paper
        ctx.beginPath()
        ctx.moveTo(-step, H + 1)
        const lift = lineH * 0.62
        for (let c = 0; c < cols; c++) ctx.lineTo(c * step - frac + half, wy[c] - lift)
        ctx.lineTo(W + step, H + 1)
        ctx.closePath()
        ctx.fill()
      }
      ctx.globalAlpha = Math.min(1, b.alpha / BODY_INK)
      const gw = cw / dpr
      const gh = ch / dpr
      const rows = b.rows
      const nRows = rows.length
      const next = i < BANDS - 1 ? bands[i + 1] : null
      for (let c = 0; c < cols; c++) {
        const x = c * step - frac + half
        const y = wy[c]
        if (y - lineH > H) continue
        // Rows that would sit under the next band's paper are never seen: stop there.
        const floor = next ? height(next, x, t) + lineH * 0.4 : H + lineH
        const maxRows = Math.min(nRows, Math.ceil((floor - y) / lineH) + 1)
        if (maxRows <= 0) continue
        // Shear each column to the local slope: glyphs lean with the wave, rows stay stacked.
        ctx.setTransform(dpr, ws[c] * dpr, 0, dpr, x * dpr, y * dpr)
        for (let r = 0; r < maxRows; r++) {
          const text = rows[r]
          const code = text[(start + c + r * 7) % text.length]
          const g = code & 127
          if (g === 0) continue
          const set = code & ACCENT ? SET_ACCENT : r === 0 ? SET_CREST : SET_BODY
          ctx.drawImage(atlas, g * cw, set * ch, cw, ch, -gw / 2, r * lineH - gh / 2, gw, gh)
        }
      }
      ctx.globalAlpha = 1
    }

    const snap = (v: number) => Math.round(v * dpr) / dpr

    const placeBoat = (t: number, dt: number) => {
      if (boatBand < 0) return
      const b = bands[boatBand]
      const x = bxMid + bxAmp * Math.sin(t * 0.035 + 1.1)
      const span = bw * 0.3
      const h = (height(b, x - span, t) + 2 * height(b, x, t) + height(b, x + span, t)) / 4
      const target = Math.max(-0.22, Math.min(0.22, Math.atan((height(b, x + span, t) - height(b, x - span, t)) / (2 * span)) * 0.85))
      if (Number.isNaN(poseY) || dt <= 0) {
        poseY = h
        poseVY = 0
        poseR = target
        poseVR = 0
      } else {
        // Sub-step so a long frame can't make the springs overshoot.
        let left = Math.min(dt, 0.1)
        while (left > 0) {
          const d = Math.min(left, 1 / 120)
          ;[poseY, poseVY] = spring(poseY, poseVY, h, 3.4, d)
          ;[poseR, poseVR] = spring(poseR, poseVR, target, 2.6, d)
          left -= d
        }
      }
      const roll = poseR
      const tx = x - bw / 2
      const ty = poseY - WATERLINE * bs
      const tf = `translate3d(${tx.toFixed(2)}px, ${ty.toFixed(2)}px, 0) rotate(${roll.toFixed(4)}rad)`
      boat.style.transform = tf
      rail.style.transform = tf
      // Crew: upright, feet on the rolled deck line, the odd small hop of their own.
      const ox = bw / 2
      const oy = WATERLINE * bs
      const cs = Math.cos(roll)
      const sn = Math.sin(roll)
      const pitch = (CREW_PITCH * cell) / bs
      const hopH = cell * 1.6
      for (let k = 0; k < crew.length; k++) {
        const el = crew[k]
        if (!el) continue
        const lx = DECK_MID + (k - (CREW.length - 1) / 2) * pitch
        const dx = lx * bs - ox
        const dy = deckY(lx) * bs - oy
        const fx = tx + ox + dx * cs - dy * sn
        const fy = ty + oy + dx * sn + dy * cs
        const period = 3.1 + k * 0.53
        const u = (t + k * 1.37) % period
        const hop = dt > 0 && u < 0.38 ? Math.sin((Math.PI * u) / 0.38) * hopH : 0
        const left = snap(fx - (CREW_CX - CREW_VIEW.x) * cell)
        const top = snap(fy - (CREW_FEET - CREW_VIEW.y) * cell * 2 - hop)
        el.style.transform = `translate3d(${left}px, ${top}px, 0)`
      }
    }

    // Dev builds report the draw cost on the canvas (data-frame-ms="avg/max" over ~2s).
    let statN = 0
    let statSum = 0
    let statMax = 0

    const draw = (t: number, dt: number) => {
      if (!atlas) return
      const t0 = import.meta.env.DEV ? performance.now() : 0
      const split = boatBand < 0 ? BANDS : boatBand + 1
      bctx.setTransform(1, 0, 0, 1, 0, 0)
      bctx.clearRect(0, 0, back.width, back.height)
      for (let i = 0; i < split; i++) drawBand(bctx, bands[i], i, t)
      fctx.setTransform(1, 0, 0, 1, 0, 0)
      fctx.clearRect(0, 0, front.width, front.height)
      for (let i = split; i < BANDS; i++) drawBand(fctx, bands[i], i, t)
      placeBoat(t, dt)
      if (import.meta.env.DEV) {
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
    const tStart = performance.now()
    const STILL_T = 7

    const loop = (now: number) => {
      raf = requestAnimationFrame(loop)
      const dt = last ? (now - last) / 1000 : 0
      last = now
      draw((now - tStart) / 1000, dt)
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
      for (const el of [boat, rail, ...crew]) el?.classList.add('ready')
      if (reduce.matches) {
        stop()
        draw(STILL_T, 0)
      } else {
        draw((performance.now() - tStart) / 1000, 0)
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
      <div ref={railRef} className="sea-boat" aria-hidden="true">
        <BoatRail />
      </div>
      <canvas ref={frontRef} className="sea sea-front" aria-hidden="true" />
    </>
  )
}
