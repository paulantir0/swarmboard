import { useEffect, useRef, type RefObject } from 'react'

// Looking down at a river at night, drawn in Swarm Mono. The current runs left to right and
// bends around the form (the "stone"): texture is carried along the streamlines of potential
// flow past an ellipse, so ripples squeeze and quicken at its shoulders, foam gathers on its
// upstream face and slow eddies shed off its tail. Two depth layers drift at different speeds
// (deep shade under the surface ripples), and a few glints ride the current in the accent.
//
// Cost: a value-noise lattice, per-cell stream coordinates computed once per resize, a glyph
// atlas baked per theme, 24fps, paused when hidden, a single still frame for reduced motion.

const FPS = 24
const FAMILY = "'Swarm Mono', ui-monospace, monospace"
const GLYPHS = ['.', '·', '-', '∼', '≈', '°', '•']
const G_DOT = 0
const G_MID = 1
const G_DASH = 2
const G_TILDE = 3
const G_WAVE = 4
const G_FOAM = 5
const G_GLINT = 6
const LEVELS = 16 // alpha steps per glyph, per color

const U_SURFACE = 22 // px/s far from the stone
const U_DEEP = 9

// 256x256 lattice of random values; tiles, so lookups wrap with a mask.
const LAT = (() => {
  const a = new Float32Array(256 * 256)
  let s = 0x2545f491
  for (let i = 0; i < a.length; i++) {
    s ^= s << 13
    s ^= s >>> 17
    s ^= s << 5
    a[i] = (s >>> 0) / 4294967296
  }
  return a
})()

/** Smooth value noise in [0, 1]. */
function vn(x: number, y: number): number {
  const xf = Math.floor(x)
  const yf = Math.floor(y)
  let tx = x - xf
  let ty = y - yf
  tx = tx * tx * (3 - 2 * tx)
  ty = ty * ty * (3 - 2 * ty)
  const x0 = xf & 255
  const x1 = (x0 + 1) & 255
  const y0 = (yf & 255) << 8
  const y1 = ((yf + 1) & 255) << 8
  const a = LAT[y0 | x0]
  const b = LAT[y0 | x1]
  const c = LAT[y1 | x0]
  const d = LAT[y1 | x1]
  return a + (b - a) * tx + (c - a) * ty + (a - b - c + d) * tx * ty
}

const smooth = (e0: number, e1: number, x: number) => {
  const t = Math.min(1, Math.max(0, (x - e0) / (e1 - e0)))
  return t * t * (3 - 2 * t)
}

interface Eddy {
  x: number
  y: number
  dist: number
  sign: number
}

export function Water({ className, stone }: { className?: string; stone?: RefObject<HTMLElement | null> }) {
  const ref = useRef<HTMLCanvasElement>(null)

  useEffect(() => {
    const canvas = ref.current
    if (!canvas) return
    const ctx = canvas.getContext('2d')
    if (!ctx) return

    const root = document.documentElement
    const reduce = window.matchMedia('(prefers-reduced-motion: reduce)')
    const scheme = window.matchMedia('(prefers-color-scheme: dark)')

    // Grid, in device pixels per cell (ch = 2 * cw keeps the 0.6em x 1.2em cell exact so blocks tile).
    let W = 0
    let H = 0
    let dpr = 1
    let cw = 8
    let ch = 16
    let cols = 0
    let rows = 0
    let atlas: HTMLCanvasElement | null = null

    // Per cell: stream coordinates (P along, Q across), visibility, foam weight, flow velocity.
    let P = new Float32Array(0)
    let Q = new Float32Array(0)
    let M = new Float32Array(0)
    let F = new Float32Array(0)
    let VX = new Float32Array(0)
    let VY = new Float32Array(0)
    let S = new Float32Array(0) // last surface intensity, for placing glints on crests
    let J = new Float32Array(0) // per-cell dither, breaks runs of one glyph into mixed marks
    let DP = new Float32Array(0)
    let DQ = new Float32Array(0)
    let occ = new Uint8Array(0)
    let wake = new Int32Array(0)

    // Stone (ellipse around the form), CSS px.
    let sx = 0
    let sy = 0
    let sa = 0
    let sb = 0

    // Glints: a small pool riding the current.
    let nGlints = 0
    let gx = new Float32Array(0)
    let gy = new Float32Array(0)
    let gAge = new Float32Array(0)
    let gLife = new Float32Array(0)

    const eddies: Eddy[] = []
    let eddyClock = 0
    let eddySign = 1

    const isDark = () => root.dataset.theme === 'dark' || (root.dataset.theme !== 'light' && scheme.matches)

    const buildAtlas = () => {
      const css = getComputedStyle(root)
      const ink = css.getPropertyValue('--water-ink').trim() || '17, 17, 17'
      const glint = css.getPropertyValue('--water-glint').trim() || '44, 74, 124'
      const dark = isDark()
      const maxInk = dark ? 0.3 : 0.42
      const maxGlint = dark ? 0.8 : 0.62
      const a = document.createElement('canvas')
      a.width = cw * GLYPHS.length
      a.height = ch * LEVELS * 2
      const g = a.getContext('2d')
      if (!g) return null
      const fontPx = cw / 0.6
      g.font = `400 ${fontPx}px ${FAMILY}`
      g.textAlign = 'center'
      g.textBaseline = 'alphabetic'
      const m = g.measureText('≈')
      const asc = m.fontBoundingBoxAscent || fontPx * 0.92
      const desc = m.fontBoundingBoxDescent || fontPx * 0.28
      const base = (ch - (asc + desc)) / 2 + asc
      for (let set = 0; set < 2; set++) {
        const rgb = set === 0 ? ink : glint
        const max = set === 0 ? maxInk : maxGlint
        for (let l = 0; l < LEVELS; l++) {
          const alpha = max * Math.pow((l + 1) / LEVELS, 1.25)
          g.fillStyle = `rgba(${rgb}, ${alpha.toFixed(4)})`
          const y = (set * LEVELS + l) * ch + base
          for (let i = 0; i < GLYPHS.length; i++) g.fillText(GLYPHS[i], i * cw + cw / 2, y)
        }
      }
      return a
    }

    const spawnGlint = (k: number, initial: boolean) => {
      for (let tries = 0; tries < 6; tries++) {
        const x = Math.random() * W
        const y = Math.random() * H
        const c = Math.floor((x * dpr) / cw)
        const r = Math.floor((y * dpr) / ch)
        const i = r * cols + c
        if (i < 0 || i >= M.length || M[i] < 0.6) continue
        if (tries < 4 && S[i] < 0.6) continue // prefer crests
        gx[k] = x
        gy[k] = y
        gLife[k] = 1.6 + Math.random() * 2.6
        gAge[k] = initial ? Math.random() * gLife[k] : 0
        return
      }
      gLife[k] = 1
      gAge[k] = 1 // try again next frame
    }

    const layout = () => {
      const rect = canvas.getBoundingClientRect()
      W = Math.max(1, Math.round(rect.width))
      H = Math.max(1, Math.round(rect.height))
      dpr = Math.min(2, window.devicePixelRatio || 1)
      const fontCss = W < 560 ? 13 : 14
      cw = Math.max(4, Math.round(fontCss * 0.6 * dpr))
      ch = cw * 2
      canvas.width = Math.ceil(W * dpr)
      canvas.height = Math.ceil(H * dpr)
      cols = Math.ceil(canvas.width / cw)
      rows = Math.ceil(canvas.height / ch)
      const n = cols * rows

      const el = stone?.current
      if (el && el.offsetWidth > 0) {
        // offset* ignore the entrance transform; the block and canvas share the same parent box.
        const w = el.offsetWidth
        const h = el.offsetHeight
        sx = el.offsetLeft + w / 2
        sy = el.offsetTop + h / 2
        // Just wide enough to hold the block's corners, so the current runs close past it.
        sa = (w / 2) * 1.2 + 24
        sb = (h / 2) * 1.25 + 24
      } else {
        sa = 0
        sb = 0
      }

      P = new Float32Array(n)
      Q = new Float32Array(n)
      M = new Float32Array(n)
      F = new Float32Array(n)
      VX = new Float32Array(n)
      VY = new Float32Array(n)
      S = new Float32Array(n)
      J = new Float32Array(n)
      for (let i = 0; i < n; i++) J[i] = Math.random() - 0.5
      DP = new Float32Array(n)
      DQ = new Float32Array(n)
      occ = new Uint8Array(n)
      const wakeList: number[] = []
      const cssW = cw / dpr
      const cssH = ch / dpr

      for (let r = 0; r < rows; r++) {
        const y = (r + 0.5) * cssH
        for (let c = 0; c < cols; c++) {
          const i = r * cols + c
          const x = (c + 0.5) * cssW
          if (sa === 0) {
            P[i] = x
            Q[i] = y
            M[i] = 1
            VX[i] = 1
            continue
          }
          const X = (x - sx) / sa
          const Y = (y - sy) / sb
          const r2 = X * X + Y * Y
          if (r2 < 1) continue // inside the stone: M stays 0
          const inv = 1 / r2
          const inv2 = inv * inv
          // Q: stream function of flow past a cylinder (constant along streamlines).
          // P: along-stream coordinate whose spacing follows the local speed (2x at the shoulders).
          P[i] = sa * X * (1 - 0.5 * inv)
          Q[i] = sb * Y * (1 - inv)
          VX[i] = 1 - (X * X - Y * Y) * inv2
          VY[i] = (sb / sa) * (-2 * X * Y * inv2)
          // Calm margin around the form, the current filling in a little way out.
          M[i] = 0.15 + 0.85 * smooth(1.0, 1.5, r2)
          if (X < -0.1 && r2 < 1.32) F[i] = (1 - (r2 - 1) / 0.32) * Math.min(1, (-X - 0.1) / 0.55)
          if (X > 0.6 && X < 4.2 && Math.abs(Y) < 1.9) wakeList.push(i)
        }
      }
      wake = Int32Array.from(wakeList)

      nGlints = Math.round(Math.min(60, Math.max(8, (W * H) / 34000)))
      gx = new Float32Array(nGlints)
      gy = new Float32Array(nGlints)
      gAge = new Float32Array(nGlints)
      gLife = new Float32Array(nGlints)
      for (let k = 0; k < nGlints; k++) spawnGlint(k, true)
      eddies.length = 0
      eddyClock = 0
      atlas = buildAtlas()
    }

    // Eddies shed off the stone's tail, alternating sides, and drift away (a soft Karman street).
    const stepEddies = (dt: number) => {
      if (sa === 0) return
      const speed = U_SURFACE * 0.55
      const spacing = sa * 0.6
      const travel = sa * 2.6
      eddyClock -= dt
      if (eddyClock <= 0) {
        eddyClock += spacing / speed
        eddies.push({ x: sx + sa * 1.05, y: sy + eddySign * sb * 0.34, dist: 0, sign: eddySign })
        eddySign = -eddySign
      }
      for (let e = eddies.length - 1; e >= 0; e--) {
        const ed = eddies[e]
        ed.x += speed * dt
        ed.y += ed.sign * sb * 0.05 * dt
        ed.dist += speed * dt
        if (ed.dist > travel) eddies.splice(e, 1)
      }
      const sigma2 = sb * 0.5 * (sb * 0.5)
      const cssW = cw / dpr
      const cssH = ch / dpr
      for (let k = 0; k < wake.length; k++) {
        const i = wake[k]
        const x = ((i % cols) + 0.5) * cssW
        const y = (Math.floor(i / cols) + 0.5) * cssH
        let ox = 0
        let oy = 0
        for (let e = 0; e < eddies.length; e++) {
          const ed = eddies[e]
          const dx = x - ed.x
          const dy = y - ed.y
          const d2 = dx * dx + dy * dy
          if (d2 > sigma2 * 9) continue
          const life = Math.sin((Math.PI * ed.dist) / travel)
          const th = ed.sign * 1.25 * life * Math.exp(-d2 / sigma2)
          const cs = Math.cos(th)
          const sn = Math.sin(th)
          ox += dx * cs - dy * sn - dx
          oy += dx * sn + dy * cs - dy
        }
        DP[i] = ox
        DQ[i] = oy
      }
    }

    const stepGlints = (dt: number) => {
      occ.fill(0)
      const cssToCol = dpr / cw
      const cssToRow = dpr / ch
      for (let k = 0; k < nGlints; k++) {
        gAge[k] += dt
        let c = Math.floor(gx[k] * cssToCol)
        let r = Math.floor(gy[k] * cssToRow)
        let i = r * cols + c
        if (gAge[k] >= gLife[k] || c < 0 || c >= cols || r < 0 || r >= rows || M[i] < 0.3) {
          spawnGlint(k, false)
          continue
        }
        gx[k] += VX[i] * U_SURFACE * dt
        gy[k] += VY[i] * U_SURFACE * dt
        c = Math.floor(gx[k] * cssToCol)
        r = Math.floor(gy[k] * cssToRow)
        i = r * cols + c
        if (c >= 0 && c < cols && r >= 0 && r < rows) occ[i] = 1
      }
    }

    const draw = (t: number, dt: number, moving: boolean) => {
      if (!atlas) return
      const A = atlas
      ctx.setTransform(1, 0, 0, 1, 0, 0)
      ctx.clearRect(0, 0, canvas.width, canvas.height)
      if (moving) {
        stepEddies(dt)
        stepGlints(dt)
      } else {
        occ.fill(0)
      }

      const sTop = LEVELS - 1
      const us = U_SURFACE * t
      const us2 = U_SURFACE * 1.35 * t
      const ud = U_DEEP * t
      for (let r = 0; r < rows; r++) {
        const dy = r * ch
        for (let c = 0; c < cols; c++) {
          const i = r * cols + c
          const m = M[i]
          if (m === 0) continue
          const p = P[i] + DP[i]
          const q = Q[i] + DQ[i]
          const dx = c * cw

          // Slow meander: the whole surface sways a little, so streaks never just scroll.
          // Slow meanders: streak paths wander across rows and drift over time.
          const w = vn(p * 0.0028 + 7.7, q * 0.0045 - t * 0.02) - 0.5
          const pw = p + w * 40
          const qw = q + w * 96

          // Moonlit patches: where the river catches more light.
          const lum = vn(p * 0.0017 - t * 0.006 + 41.3, q * 0.0031 + 5.9)

          // Surface: a slow swell, streaks stretched along the current, and fine ripples.
          const s1 = vn((pw - us) * 0.014, qw * 0.034 + t * 0.04)
          const s2 = vn((pw - us2) * 0.05 + 31.3, qw * 0.07 - t * 0.07)
          const s3 = vn((pw - us * 0.8) * 0.0075 + 5.1, qw * 0.016 + t * 0.02)
          const s = s1 * 0.42 + s2 * 0.28 + s3 * 0.3 // mean 0.5, sd ~0.125
          S[i] = s
          if (occ[i]) continue

          const f = F[i]
          if (f > 0) {
            const fo = f * (vn(p * 0.11 - t * 0.9, q * 0.13 + t * 0.4) * 1.6 - 0.35)
            if (fo > 0.42) {
              const lv = Math.min(sTop, Math.floor((0.25 + 0.5 * fo) * sTop))
              ctx.drawImage(A, (fo > 0.7 ? G_FOAM : G_MID) * cw, lv * ch, cw, ch, dx, dy, cw, ch)
              continue
            }
          }

          // Tone sets the glyph (quantiles of s: ~25% open water, ~8% crests); alpha stays
          // in a narrow band so the field reads as one woven surface rather than scattered marks.
          const k = s + J[i] * 0.03
          if (k < 0.413) {
            // Between ripples, a fainter layer of silt drifts slower: depth by parallax.
            const d = vn((pw - ud) * 0.021 + 91.1, qw * 0.06 + 13.7)
            if (d < 0.58) continue
            const lv = Math.min(4, Math.floor((d - 0.58) * 12 * m * (0.6 + 0.8 * lum)))
            ctx.drawImage(A, (d > 0.7 ? G_MID : G_DOT) * cw, lv * ch, cw, ch, dx, dy, cw, ch)
            continue
          }
          const g = k < 0.491 ? G_DOT : k < 0.559 ? G_MID : k < 0.615 ? G_DASH : k < 0.68 ? G_TILDE : G_WAVE
          const tone = Math.min(1, (k - 0.413) / 0.3)
          const a = (0.45 + 0.55 * tone) * m * (0.55 + 0.9 * lum)
          const lv = Math.max(0, Math.min(sTop, Math.floor(a * sTop)))
          ctx.drawImage(A, g * cw, lv * ch, cw, ch, dx, dy, cw, ch)
        }
      }

      if (!moving) return
      const cssToCol = dpr / cw
      const cssToRow = dpr / ch
      for (let k = 0; k < nGlints; k++) {
        const c = Math.floor(gx[k] * cssToCol)
        const r = Math.floor(gy[k] * cssToRow)
        if (c < 0 || c >= cols || r < 0 || r >= rows) continue
        const e = Math.sin((Math.PI * gAge[k]) / gLife[k])
        const a = e * e * M[r * cols + c]
        if (a < 0.08) continue
        const lv = Math.min(sTop, Math.floor(a * sTop))
        ctx.drawImage(A, (e > 0.8 ? G_GLINT : G_MID) * cw, (LEVELS + lv) * ch, cw, ch, c * cw, r * ch, cw, ch)
      }
    }

    let raf = 0
    let last = 0
    let running = false
    let ready = false
    let disposed = false
    const t0 = performance.now()
    const STILL_T = 37

    const loop = (now: number) => {
      raf = requestAnimationFrame(loop)
      if (now - last < 1000 / FPS - 2) return
      const dt = last ? Math.min(0.1, (now - last) / 1000) : 1 / FPS
      last = now
      draw((now - t0) / 1000, dt, true)
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
      if (reduce.matches) {
        stop()
        draw(STILL_T, 0, false)
      } else {
        draw((performance.now() - t0) / 1000, 0, true)
        play()
      }
    }

    let pending = 0
    const schedule = () => {
      cancelAnimationFrame(pending)
      pending = requestAnimationFrame(refresh)
    }

    const onVisibility = () => (document.hidden ? stop() : play())
    const onTheme = () => {
      if (!ready) return
      atlas = buildAtlas()
      if (reduce.matches) draw(STILL_T, 0, false)
    }

    const ro = new ResizeObserver(schedule)
    ro.observe(canvas)
    if (stone?.current) ro.observe(stone.current)
    const mo = new MutationObserver(onTheme)
    mo.observe(root, { attributes: true, attributeFilter: ['data-theme'] })
    document.addEventListener('visibilitychange', onVisibility)
    reduce.addEventListener('change', refresh)
    scheme.addEventListener('change', onTheme)

    // Draw only once Swarm Mono is in, so the atlas is baked from the real glyphs.
    const fonts = document.fonts
    const loaded = fonts
      ? Promise.all([fonts.load(`400 14px ${FAMILY}`, '.·-∼≈°•'), fonts.load(`600 14px ${FAMILY}`, 'swarmboard')])
      : Promise.resolve()
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
      scheme.removeEventListener('change', onTheme)
    }
  }, [stone])

  return <canvas ref={ref} className={className} aria-hidden="true" />
}
