import { useEffect, useRef, type RefObject } from 'react'
import { BOAT_H, BOAT_W, Boat, WATERLINE } from './Boat'
import { SNIPPETS, isMarker } from './seaText'

// A sea of code-review noise. Rows of snippets scroll sideways at depth-dependent speeds; each
// character rides a travelling swell (a sum of three sines), so the text itself undulates. Back
// rows are small and faint near the horizon, front rows large and darker. The boat floats on the
// same swell function at its row, split between two canvases so nearer rows wash over its hull.
//
// Cost: one glyph atlas per font size (ink + accent), rows as typed arrays of glyph indices,
// 24fps, paused when hidden, one still frame for reduced motion.

const FPS = 24
const FAMILY = "'Swarm Mono', ui-monospace, monospace"
const TAU = Math.PI * 2

const lerp = (a: number, b: number, t: number) => a + (b - a) * t
const clamp01 = (t: number) => Math.min(1, Math.max(0, t))

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

interface Atlas {
  img: HTMLCanvasElement
  cw: number // device px
  ch: number
}

export function Sea({ sky }: { sky: RefObject<HTMLElement | null> }) {
  const backRef = useRef<HTMLCanvasElement>(null)
  const frontRef = useRef<HTMLCanvasElement>(null)
  const boatRef = useRef<HTMLDivElement>(null)

  useEffect(() => {
    const back = backRef.current
    const front = frontRef.current
    const boat = boatRef.current
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
    let n = 0
    let rowY = new Float32Array(0)
    let rowSize = new Uint8Array(0)
    let rowAlpha = new Float32Array(0)
    let rowSpeed = new Float32Array(0)
    let rowAmp = new Float32Array(0)
    let rowK1 = new Float32Array(0)
    let rowK2 = new Float32Array(0)
    let rowK3 = new Float32Array(0)
    let rowPhase = new Float32Array(0)
    let texts: Uint8Array[] = []
    const atlases = new Map<number, Atlas>()

    // Boat
    let boatRow = -1
    let bw = 0
    let bScale = 1
    let bxMid = 0
    let bxAmp = 0

    const buildAtlases = () => {
      atlases.clear()
      const css = getComputedStyle(root)
      const ink = css.getPropertyValue('--sea-ink').trim() || '17, 17, 17'
      const accent = css.getPropertyValue('--sea-accent').trim() || '44, 74, 124'
      const sizes = new Set(rowSize)
      for (const size of sizes) {
        const fontPx = size * dpr
        const cw = Math.max(2, Math.round(size * 0.6 * dpr))
        const ch = Math.ceil(size * 1.4 * dpr)
        const img = document.createElement('canvas')
        img.width = cw * CHARS.length
        img.height = ch * 2
        const g = img.getContext('2d')
        if (!g) continue
        g.font = `400 ${fontPx}px ${FAMILY}`
        g.textAlign = 'center'
        g.textBaseline = 'middle'
        for (let set = 0; set < 2; set++) {
          g.fillStyle = `rgb(${set ? accent : ink})`
          for (let i = 1; i < CHARS.length; i++) g.fillText(CHARS[i], i * cw + cw / 2, set * ch + ch / 2)
        }
        atlases.set(size, { img, cw, ch })
      }
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
      const seaTop = Math.min(H - 48, Math.max(H * 0.48, skyBottom + (small ? 20 : 28)))
      const sMin = small ? 7 : 8
      const sMax = small ? 13 : 17
      const ampMax = small ? 8 : 13

      const ys: number[] = []
      for (let y = seaTop; y < H + sMax; ) {
        ys.push(y)
        const z = clamp01((y - seaTop) / Math.max(1, H - seaTop))
        y += lerp(sMin, sMax, z) * 1.32
      }
      n = ys.length
      rowY = Float32Array.from(ys)
      rowSize = new Uint8Array(n)
      rowAlpha = new Float32Array(n)
      rowSpeed = new Float32Array(n)
      rowAmp = new Float32Array(n)
      rowK1 = new Float32Array(n)
      rowK2 = new Float32Array(n)
      rowK3 = new Float32Array(n)
      rowPhase = new Float32Array(n)
      texts = []
      const rand = mulberry32(0x5eab0a7)
      const dark = isDark()
      for (let i = 0; i < n; i++) {
        const z = clamp01((rowY[i] - seaTop) / Math.max(1, H - seaTop))
        const size = Math.round(lerp(sMin, sMax, z))
        rowSize[i] = size
        rowAlpha[i] = lerp(0.07, dark ? 0.36 : 0.42, Math.pow(z, 0.75))
        rowSpeed[i] = lerp(5, 30, z) * (0.85 + 0.3 * rand())
        rowAmp[i] = lerp(1, ampMax, Math.pow(z, 1.25))
        rowK1[i] = TAU / lerp(150, 560, z)
        rowK2[i] = TAU / lerp(70, 250, z)
        rowK3[i] = TAU / lerp(34, 110, z)
        rowPhase[i] = i * 0.62
        texts.push(rowText(rand, Math.ceil(W / (size * 0.6)) * 2 + 40))
      }

      // Boat: sized to the viewport, on a row deep enough that its masthead clears the copy.
      bw = small ? Math.min(200, W * 0.52) : Math.min(360, Math.max(220, W * 0.25))
      bScale = bw / BOAT_W
      const above = WATERLINE * bScale
      if (small) {
        bxMid = W * 0.6
        bxAmp = W * 0.06
      } else {
        bxMid = Math.max(skyRight + bw * 0.6, W * 0.68)
        bxAmp = Math.min(W * 0.08, Math.max(0, W - bw * 0.6 - bxMid))
      }
      const overlaps = bxMid - bxAmp - bw / 2 < skyRight
      boatRow = -1
      for (let i = 0; i < n; i++) {
        const z = (rowY[i] - seaTop) / Math.max(1, H - seaTop)
        if (z < (small ? 0.3 : 0.26)) continue
        if (overlaps && rowY[i] - above - rowAmp[i] < skyBottom + 6) continue
        boatRow = i
        break
      }
      // Too short for the boat to clear the copy (landscape phones): leave it ashore.
      if (boatRow >= 0 && rowY[boatRow] + 12 > H) boatRow = -1
      boat.style.width = `${bw}px`
      boat.style.height = `${BOAT_H * bScale}px`
      boat.style.transformOrigin = `${bw / 2}px ${WATERLINE * bScale}px`
      boat.style.display = boatRow < 0 ? 'none' : ''

      buildAtlases()
    }

    const swell = (i: number, x: number, t: number) => {
      const ph = rowPhase[i]
      return (
        rowAmp[i] *
        (0.6 * Math.sin(x * rowK1[i] + t * 0.62 + ph) +
          0.28 * Math.sin(x * rowK2[i] - t * 0.5 + ph * 1.7) +
          0.12 * Math.sin(x * rowK3[i] + t * 1.5 + ph * 0.6))
      )
    }

    const drawRows = (ctx: CanvasRenderingContext2D, from: number, to: number, t: number) => {
      ctx.setTransform(1, 0, 0, 1, 0, 0)
      ctx.clearRect(0, 0, back.width, back.height)
      for (let i = from; i < to; i++) {
        const atlas = atlases.get(rowSize[i])
        if (!atlas) continue
        const { img, cw, ch } = atlas
        const text = texts[i]
        const len = text.length
        const step = cw / dpr
        const off = rowSpeed[i] * t
        const start = Math.floor(off / step)
        const frac = off - start * step
        const cols = Math.ceil(W / step) + 1
        const base = rowY[i] * dpr - ch / 2
        ctx.globalAlpha = rowAlpha[i]
        for (let c = 0; c < cols; c++) {
          const code = text[(start + c) % len]
          const g = code & 127
          if (g === 0) continue
          const x = c * step - frac
          const y = base + swell(i, x + step / 2, t) * dpr
          ctx.drawImage(img, g * cw, code >> 7 ? ch : 0, cw, ch, Math.round(x * dpr), Math.round(y), cw, ch)
        }
      }
      ctx.globalAlpha = 1
    }

    const placeBoat = (t: number) => {
      if (boatRow < 0) return
      const x = bxMid + bxAmp * Math.sin(t * 0.045 + 1.1)
      const h = swell(boatRow, x, t)
      const span = bw * 0.32
      const slope = (swell(boatRow, x + span, t) - swell(boatRow, x - span, t)) / (2 * span)
      const roll = Math.max(-0.22, Math.min(0.22, Math.atan(slope) * 0.9 + 0.02 * Math.sin(t * 0.8)))
      const y = rowY[boatRow] + h - WATERLINE * bScale
      boat.style.transform = `translate3d(${(x - bw / 2).toFixed(1)}px, ${y.toFixed(1)}px, 0) rotate(${roll.toFixed(4)}rad)`
    }

    const draw = (t: number) => {
      if (!atlases.size) return
      const split = boatRow < 0 ? n : boatRow + 1
      drawRows(bctx, 0, split, t)
      drawRows(fctx, split, n, t)
      placeBoat(t)
    }

    let raf = 0
    let last = 0
    let running = false
    let ready = false
    let disposed = false
    const t0 = performance.now()
    const STILL_T = 21

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
    const onTheme = () => {
      if (!ready) return
      refresh()
    }

    const ro = new ResizeObserver(schedule)
    ro.observe(back)
    if (sky.current) ro.observe(sky.current)
    const mo = new MutationObserver(onTheme)
    mo.observe(root, { attributes: true, attributeFilter: ['data-theme'] })
    document.addEventListener('visibilitychange', onVisibility)
    reduce.addEventListener('change', refresh)
    scheme.addEventListener('change', onTheme)

    const fonts = document.fonts
    const loaded = fonts ? fonts.load(`400 14px ${FAMILY}`, SNIPPETS.slice(0, 20).join('')) : Promise.resolve()
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
  }, [sky])

  return (
    <>
      <canvas ref={backRef} className="sea" aria-hidden="true" />
      <div ref={boatRef} className="sea-boat" aria-hidden="true">
        <Boat />
      </div>
      <canvas ref={frontRef} className="sea sea-front" aria-hidden="true" />
    </>
  )
}
