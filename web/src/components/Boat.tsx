// A hand-inked sailboat in the Swarm Mono monoline style: a curved hull whose sheer rises at bow
// and stern, plank lines, a low rail the crew stand behind, a bellied mainsail carrying the logo's
// three waves, a jib, mast, pennant, forestay and backstay. Strokes don't scale with the boat, and
// their weights vary a little (hull heaviest, rigging lightest) for a hand-drawn feel.
//
// Viewbox 400 x 250, bow to the right; the waterline is y = WATERLINE. The boat is drawn in two
// layers so the crew (Crew.tsx) can stand on deck between them: <Boat /> behind, <BoatRail /> in
// front (the rail and its stanchions, through whose gaps the crew's feet show).

export const BOAT_W = 400
export const BOAT_H = 250
export const WATERLINE = 200
/** Masthead, for clearance above the copy. */
export const BOAT_TOP = 14
/** Usable deck for the crew (viewbox x), and its middle. */
export const DECK_X0 = 64
export const DECK_X1 = 344
export const DECK_MID = (DECK_X0 + DECK_X1) / 2

type Cubic = [number, number, number, number, number, number, number, number]

const SHEER: Cubic = [30, 148, 110, 170, 290, 172, 374, 138]
const RAIL: Cubic = [40, 133, 114, 153, 288, 155, 364, 125]

function sample(c: Cubic, n = 96): [number, number][] {
  const pts: [number, number][] = []
  for (let i = 0; i <= n; i++) {
    const t = i / n
    const u = 1 - t
    const a = u * u * u
    const b = 3 * u * u * t
    const d = 3 * u * t * t
    const e = t * t * t
    pts.push([a * c[0] + b * c[2] + d * c[4] + e * c[6], a * c[1] + b * c[3] + d * c[5] + e * c[7]])
  }
  return pts
}

function yAt(pts: [number, number][], x: number): number {
  for (let i = 1; i < pts.length; i++) {
    const [x1, y1] = pts[i]
    if (x1 >= x) {
      const [x0, y0] = pts[i - 1]
      return y0 + ((y1 - y0) * (x - x0)) / Math.max(1e-6, x1 - x0)
    }
  }
  return pts[pts.length - 1][1]
}

const SHEER_PTS = sample(SHEER)
const RAIL_PTS = sample(RAIL)

/** Deck height (viewbox y) at viewbox x: the sheer line the crew stand on. */
export const deckY = (x: number) => yAt(SHEER_PTS, x)

const d = (c: Cubic) => `M${c[0]} ${c[1]}C${c[2]} ${c[3]} ${c[4]} ${c[5]} ${c[6]} ${c[7]}`

const STANCHIONS = [52, 84, 116, 148, 180, 226, 258, 290, 322, 352].map((x) => {
  const top = yAt(RAIL_PTS, x)
  const foot = yAt(SHEER_PTS, x + 1.5)
  return `M${x} ${top.toFixed(1)}L${(x + 1.5).toFixed(1)} ${(foot - 0.6).toFixed(1)}`
})

const WAVE = 'c4.1-3.4 7.7-3.6 11.2-.2s7.4 3.3 11.4-.3'

export function Boat() {
  return (
    <svg className="boat" viewBox={`0 0 ${BOAT_W} ${BOAT_H}`} width="100%" height="100%" aria-hidden="true" focusable="false">
      {/* standing rigging */}
      <path className="ln w1" d="M198.6 17.5L34.4 145" />
      <path className="ln w1" d="M201.4 17.5L392 131.6" />
      {/* pennant */}
      <path className="ln w14 pennant" d="M199.6 15.2C207 16.4 214.2 18.8 222.4 22.4C214.6 24.2 207.2 25.8 200 28.4" />
      {/* mainsail, bellied, with the logo's waves */}
      <path className="ln w16 sail" d="M196.4 24.6C164.2 44.6 104.8 98.6 68.8 139.6C111.4 142.8 156.6 145.4 196.2 145.2C196.8 104.8 196.2 64.6 196.4 24.6Z" />
      <path className="ln w1 seam" d="M196.2 66.4C170.2 72.8 141.6 86.4 116.6 103.2" />
      <path className="ln w1 seam" d="M196.2 106.2C163.4 110.6 128.2 119.6 92.4 128.8" />
      <path className="ln w16" d={`M156.4 74.6${WAVE}`} />
      <path className="ln w16" d={`M155.6 86.4${WAVE}`} />
      <path className="ln w16" d={`M154.8 98.2${WAVE}`} />
      {/* jib */}
      <path className="ln w16 sail" d="M209.6 26.2L363.4 130.4C321.6 136.6 262.8 140.2 214.6 141.2C218.4 104.4 216.2 64.2 209.6 26.2Z" />
      {/* mast and boom */}
      <path className="ln w2" d="M200.2 168.6C199.6 120.4 200.6 68.2 199.6 14.6" />
      <path className="ln w18" d="M200.6 146.8L62.4 141.6" />
      {/* hull */}
      <path className="ln w22 hull" d="M30 148C110 170 290 172 374 138C362.4 168.4 340.6 191.8 306.2 204.2C248.4 220.4 150.6 222.2 98.2 214.2C66.4 208.6 46.2 190.4 38.4 174.4C34.2 166 31.2 157.4 30 148Z" />
      <path className="ln w1 faint" d="M32.6 146.4C112.4 168.2 289.6 170.2 371.6 136.6" />
      {/* planks */}
      <path className="ln w12" d="M41.4 171.4C120.4 189.6 287.6 191.4 357.8 161.6" />
      <path className="ln w12" d="M62.8 194.2C140.4 208.6 268.2 208.4 331.4 188.4" />
      {/* bowsprit */}
      <path className="ln w2" d="M371.4 138.6L394.6 131.2" />
    </svg>
  )
}

/** The rail and stanchions, drawn in front of the crew. */
export function BoatRail() {
  return (
    <svg className="boat" viewBox={`0 0 ${BOAT_W} ${BOAT_H}`} width="100%" height="100%" aria-hidden="true" focusable="false">
      <path className="ln w16" d={d(RAIL)} />
      {STANCHIONS.map((s) => (
        <path key={s} className="ln w14" d={s} />
      ))}
    </svg>
  )
}
