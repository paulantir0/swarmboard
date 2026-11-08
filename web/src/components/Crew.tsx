// The crew: six Claude-style critters, one sprite recoloured, each with an accessory. The sprite
// is built like the terminal original, from quadrant blocks, so a cell is 1 wide by 2 tall: the
// SVG stretches a cell grid to w x 2w CSS px (preserveAspectRatio="none", crispEdges). Sea.tsx
// keeps them upright and places their feet on the deck. Frame swaps (steps timing): blink, a
// two-frame leg shuffle, a waving nub, a fluttering flag.
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

/** View box, in cells: the pole and flag to the left and above, the spyglass to the right. */
export const CREW_VIEW = { x: -1, y: -3, w: 23, h: 9 }
/** Cell column of the body's centre, and the cell row at the bottom of the feet. */
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
function Px({ cells, className }: { cells: Cell[]; className?: string }) {
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
    <g className={className}>
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
      // Tricorn: a wide brim, upturned corners, a crown, and a skull.
      return (
        <>
          <Px className="px-hat" cells={[...span(-1, 2, 15), [2, -2], ...span(-2, 5, 12), [15, -2]]} />
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
      // Band over the head, cups over the nubs' shoulders.
      return <Px className="px-gear" cells={[...span(-1, 2, 15), [1, 0], [2, 0], [1, 1], [2, 1], [15, 0], [16, 0], [15, 1], [16, 1]]} />
    case 'spyglass':
      return (
        <>
          <Px className="px-brass" cells={[[17, 2], [18, 2], [19, 2]]} />
          <Px className="px-lens" cells={[[20, 2], [20, 1]]} />
        </>
      )
    case 'flag':
      return (
        <>
          <Px className="px-pole" cells={[[0, -3], [0, -2], [0, -1], [0, 0], [0, 1], [0, 2], [0, 3]]} />
          <Px className="px-flag px-fa" cells={[...span(-3, 1, 4), ...span(-2, 1, 3)]} />
          <Px className="px-flag px-fb" cells={[...span(-3, 1, 3), ...span(-2, 1, 4)]} />
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
    <svg
      viewBox={`${x} ${y} ${w} ${h}`}
      preserveAspectRatio="none"
      width="100%"
      height="100%"
      shapeRendering="crispEdges"
      className="px"
      style={style}
      aria-hidden="true"
      focusable="false"
    >
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
