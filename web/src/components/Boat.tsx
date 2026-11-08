// A long, low, hand-drawn sailing boat. Monoline ink strokes (non-scaling, so the weight matches
// Swarm Mono at any size), paper fills so it hides the sea behind it, the logo's waves on the
// mainsail. Viewbox 320 x 190; the waterline sits at y = WATERLINE. The crew is a separate pixel
// layer (Crew.tsx) that stands on the deck line below.

export const BOAT_W = 320
export const BOAT_H = 190
export const WATERLINE = 150
/** Deck span (viewbox x) the crew can stand on, and the top of the pennant (for clearance). */
export const DECK_X0 = 14
export const DECK_X1 = 306
export const BOAT_TOP = 22

// Gunwale: cubic from (8,122) via (90,128), (230,128) to (312,118). Sampled for the crew's feet.
const G = [8, 122, 90, 128, 230, 128, 312, 118]
const GUNWALE = (() => {
  const pts: [number, number][] = []
  for (let i = 0; i <= 64; i++) {
    const t = i / 64
    const u = 1 - t
    const a = u * u * u
    const b = 3 * u * u * t
    const c = 3 * u * t * t
    const d = t * t * t
    pts.push([a * G[0] + b * G[2] + c * G[4] + d * G[6], a * G[1] + b * G[3] + c * G[5] + d * G[7]])
  }
  return pts
})()

/** Deck height (viewbox y) at viewbox x. */
export function deckY(x: number): number {
  for (let i = 1; i < GUNWALE.length; i++) {
    const [x1, y1] = GUNWALE[i]
    if (x1 >= x) {
      const [x0, y0] = GUNWALE[i - 1]
      return y0 + ((y1 - y0) * (x - x0)) / Math.max(1e-6, x1 - x0)
    }
  }
  return GUNWALE[GUNWALE.length - 1][1]
}

export function Boat() {
  return (
    <svg className="boat" viewBox={`0 0 ${BOAT_W} ${BOAT_H}`} width="100%" height="100%" aria-hidden="true" focusable="false">
      {/* pennant at the masthead */}
      <path className="ln pennant" d="M160.2 23.5C167 24.6 173.6 26.4 180.4 29.6C173.4 31 166.8 32.4 160.4 34.5" />
      {/* mast */}
      <path className="ln" d="M161.2 126C160.6 98 161.1 62 160.2 22.5" />
      {/* jib */}
      <path className="ln sail" d="M156.8 37.2C141.4 61.8 121.4 91.6 99.6 117.2C119.6 118.2 139.8 118 156.2 117.2C156.9 90.4 156.4 62.6 156.8 37.2Z" />
      {/* mainsail, with the logo's waves */}
      <path className="ln sail" d="M164.4 28.8C192.2 53.6 216.4 85.2 231.2 116.6C209.6 117.8 187.4 117.6 165.2 116.6C165.8 88.6 164.8 58.8 164.4 28.8Z" />
      <path className="ln" d="M173.2 80.6c4.1-3.4 7.7-3.6 11.2-.2s7.4 3.3 11.4-.3" />
      <path className="ln" d="M174 92.4c4.1-3.4 7.7-3.6 11.2-.2s7.4 3.3 11.4-.3" />
      <path className="ln" d="M174.6 104.2c4.1-3.4 7.7-3.6 11.2-.2s7.4 3.3 11.4-.3" />
      {/* hull */}
      <path className="ln hull" d="M8 122C90 128 230 128 312 118C300.4 135.6 286.2 150.2 268.4 160.2C200.6 167.4 120.2 167.6 53.6 160.4C34.2 150.6 19.8 137.8 8 122Z" />
      <path className="ln faint" d="M10.8 121.2C92 127.2 228 127.4 309.6 117.4" />
      <path className="ln" d="M22.4 137.8C110 146.2 220 146.4 296.6 133.6" />
      <circle className="ln port" cx="100" cy="153.6" r="3" />
      <circle className="ln port" cx="160.4" cy="155.2" r="3" />
      <circle className="ln port" cx="220.6" cy="153.4" r="3" />
    </svg>
  )
}
