// The crew: six pixel critters, one sprite recoloured, each with a pixel accessory. Drawn as SVG
// rects with crispEdges and sized in whole pixels per cell, so they stay sharp; they are kept
// upright (only translated, to whole pixels) while the boat rolls under them. Animation is frame
// swapping with steps() timing: blink, a two-frame leg shuffle, a waving nub, a fluttering flag.
//
// Sprite, 18 x 5 cells (# body, E eye):
//   ...############...
//   ...##E######E##...
//   .################.
//   ...############...
//   ....#.#....#.#....

import type { CSSProperties } from 'react'

type Cell = [number, number]
type Kind = 'flag' | 'bow' | 'pirate' | 'headphones' | 'waver' | 'spyglass'

/** View box, in cells: room for the flag above and the spyglass to the right. */
export const CREW_VIEW = { x: -1, y: -6, w: 22, h: 12 }
/** Cell column of the body's centre, and the row of the feet's bottom edge. */
export const CREW_CX = 9
export const CREW_FEET = 6
/** Cells from one critter to the next (16 wide plus a gap). */
export const CREW_PITCH = 17

export const CREW: { kind: Kind; color: string }[] = [
  { kind: 'flag', color: 'sage' },
  { kind: 'bow', color: 'mustard' },
  { kind: 'pirate', color: 'orange' },
  { kind: 'headphones', color: 'plum' },
  { kind: 'waver', color: 'slate' },
  { kind: 'spyglass', color: 'blue' },
]

const span = (y: number, x0: number, x1: number): Cell[] => Array.from({ length: x1 - x0 + 1 }, (_, i) => [x0 + i, y])

/** Merge cells into one rect per horizontal run. */
function Px({ cells, className, style }: { cells: Cell[]; className?: string; style?: CSSProperties }) {
  const rows = new Map<number, number[]>()
  for (const [x, y] of cells) rows.set(y, [...(rows.get(y) ?? []), x])
  const rects: [number, number, number][] = []
  for (const [y, xs] of rows) {
    xs.sort((a, b) => a - b)
    let start = xs[0]
    let prev = xs[0]
    for (let i = 1; i <= xs.length; i++) {
      if (i < xs.length && xs[i] === prev + 1) {
        prev = xs[i]
        continue
      }
      rects.push([start, y, prev - start + 1])
      if (i < xs.length) start = prev = xs[i]
    }
  }
  return (
    <g className={className} style={style}>
      {rects.map(([x, y, w]) => (
        <rect key={`${x},${y}`} x={x} y={y} width={w} height={1} />
      ))}
    </g>
  )
}

const BODY: Cell[] = [...span(0, 3, 14), ...span(1, 3, 14), ...span(2, 2, 15), ...span(3, 3, 14)]
const NUB_L: Cell[] = [[1, 2]]
const NUB_R: Cell[] = [[16, 2]]
const EYES: Cell[] = [
  [5, 1],
  [12, 1],
]
// Leg shuffle: one pair reaches the deck while the other lifts a row.
const LEGS_A: Cell[] = [[4, 4], [4, 5], [11, 4], [11, 5], [6, 4], [13, 4]]
const LEGS_B: Cell[] = [[6, 4], [6, 5], [13, 4], [13, 5], [4, 4], [11, 4]]

function Accessory({ kind }: { kind: Kind }) {
  switch (kind) {
    case 'pirate':
      return (
        <>
          <Px className="px-hat" cells={[...span(-1, 2, 15), [2, -2], ...span(-2, 4, 13), [15, -2], ...span(-3, 6, 11)]} />
          <Px className="px-skull" cells={[[8, -2], [9, -2]]} />
        </>
      )
    case 'bow':
      return (
        <>
          <Px className="px-bow" cells={[[10, -2], [11, -2], [13, -2], [14, -2], [10, -1], [11, -1], [13, -1], [14, -1]]} />
          <Px className="px-bow-knot" cells={[[12, -1]]} />
        </>
      )
    case 'headphones':
      return <Px className="px-gear" cells={[...span(-2, 3, 14), [2, -1], [15, -1], [1, 0], [2, 0], [1, 1], [2, 1], [15, 0], [16, 0], [15, 1], [16, 1]]} />
    case 'spyglass':
      return (
        <>
          <Px className="px-brass" cells={[[17, 2], [18, 2]]} />
          <Px className="px-lens" cells={[[19, 1], [19, 2], [19, 3]]} />
        </>
      )
    case 'flag':
      return (
        <>
          <Px className="px-pole" cells={Array.from({ length: 9 }, (_, i): Cell => [0, -6 + i])} />
          <Px className="px-flag px-fa" cells={[...span(-6, 1, 4), ...span(-5, 1, 4), ...span(-4, 1, 3)]} />
          <Px className="px-flag px-fb" cells={[...span(-6, 1, 3), ...span(-5, 1, 4), ...span(-4, 1, 4)]} />
        </>
      )
    default:
      return null
  }
}

export function Critter({ kind, color, delay }: { kind: Kind; color: string; delay: number }) {
  const style = { '--crew-c': `var(--crew-${color})`, '--d': `${-delay}s`, '--d2': `${-delay * 0.37}s` } as CSSProperties
  const { x, y, w, h } = CREW_VIEW
  return (
    <svg viewBox={`${x} ${y} ${w} ${h}`} width="100%" height="100%" shapeRendering="crispEdges" className="px" style={style} aria-hidden="true" focusable="false">
      <Px className="px-body" cells={[...BODY, ...NUB_L, ...(kind === 'waver' ? [] : NUB_R)]} />
      {kind === 'waver' ? (
        <>
          <Px className="px-body px-nub px-fa" cells={NUB_R} />
          <Px className="px-body px-nub px-fb" cells={[[16, 1]]} />
        </>
      ) : null}
      <Px className="px-eye" cells={EYES} />
      <Px className="px-body px-legs px-fa" cells={LEGS_A} />
      <Px className="px-body px-legs px-fb" cells={LEGS_B} />
      <Accessory kind={kind} />
    </svg>
  )
}
