// A small hand-drawn sailing boat with a crew of agents. Monoline ink strokes (non-scaling, so the
// weight matches Swarm Mono at any size), paper fills so it hides the sea behind it, the logo's
// waves on the mainsail. Viewbox 240 x 200; the waterline sits at y = WATERLINE.

export const BOAT_W = 240
export const BOAT_H = 200
export const WATERLINE = 156

type Crew = 'pirate' | 'spyglass' | 'flag' | 'bow' | 'headphones' | 'wave'

const BODY = 'M-10 2C-10.4-6.5-5.6-11.2 0-11.2C5.8-11.1 10.3-6.7 10 2.2C9.8 7.8 5.2 10.4-.2 10.3C-5.6 10.2-9.8 7.6-10 2Z'

function Critter({ x, y, color, kind, delay }: { x: number; y: number; color: string; kind: Crew; delay: number }) {
  const eyes = { animationDelay: `${delay}s` }
  return (
    <g transform={`translate(${x} ${y}) scale(1.18)`} className={`crew crew-${kind}`}>
      {/* arms behind the body */}
      {kind !== 'flag' ? <path className="ln" d="M-9.5 3.5l-4.2 3.6" /> : null}
      {kind !== 'wave' && kind !== 'flag' && kind !== 'spyglass' ? <path className="ln" d="M9.6 3.4l4.1 3.8" /> : null}
      <path className="ln" d={BODY} style={{ fill: `var(--crew-${color})` }} />
      {kind === 'spyglass' ? (
        <>
          <circle className="eye" cx="-3.6" cy="-2" r="1.4" />
          <path className="ln" d="M2 1.5l5-2.5" />
          <path className="ln glass" d="M3.4-3.1l15.2-4.3l1.3 4.6l-15.2 4.3z" />
          <path className="ln" d="M15.3-6.9l1.3 4.6" />
        </>
      ) : (
        <g className="eyes" style={eyes}>
          <circle className="eye" cx="-3.6" cy="-2" r="1.4" />
          <circle className="eye" cx="3.6" cy="-2" r="1.4" />
        </g>
      )}
      {kind === 'pirate' ? (
        <>
          <path className="ln hat" d="M-12.5-7.2C-10.6-12.8-6-16.3.2-16.2C6.3-16.1 10.8-12.6 12.6-7.1C7.8-9.6-7.6-9.7-12.5-7.2Z" />
          <circle className="hat-mark" cx="0" cy="-12.2" r="1.3" />
        </>
      ) : null}
      {kind === 'bow' ? (
        <g transform="translate(5.5 -10.5) rotate(-12)">
          <path className="ln bow" d="M0 0L-5.6-3.6L-5.2 3.3Z" />
          <path className="ln bow" d="M0 0L5.6-3.5L5.3 3.4Z" />
          <circle className="bow" cx="0" cy="0" r="1.6" />
        </g>
      ) : null}
      {kind === 'headphones' ? (
        <>
          <path className="ln" d="M-10.8-1.5C-11.6-12.8 11.4-13.1 10.9-1.6" />
          <rect className="ln cup" x="-13.6" y="-5" width="5" height="7.6" rx="2.2" />
          <rect className="ln cup" x="8.6" y="-5" width="5" height="7.6" rx="2.2" />
        </>
      ) : null}
      {kind === 'flag' ? (
        <>
          <path className="ln" d="M-9.5 3.5l-4.2 3.6" />
          <path className="ln" d="M9.4 1.2l5.4-4.4" />
          <path className="ln" d="M15.2 7.5L14.6-25" />
          <path className="ln flag" d="M14.7-25C19-25.8 23.4-22.9 27.6-23.6L26.4-16.9C22.4-16.2 18.5-18.9 14.8-18.1Z" />
        </>
      ) : null}
      {kind === 'wave' ? (
        <g className="waver">
          <path className="ln" d="M9.2 0.8L15.6-8.4" />
          <circle className="ln hand" cx="16.2" cy="-9.6" r="1.9" />
        </g>
      ) : null}
    </g>
  )
}

export function Boat() {
  return (
    <svg className="boat" viewBox={`0 0 ${BOAT_W} ${BOAT_H}`} width="100%" height="100%" aria-hidden="true" focusable="false">
      {/* pennant at the masthead */}
      <path className="ln pennant" d="M120.2 21.5C127 22.6 133.6 24.4 140.4 27.6C133.4 29 126.8 30.4 120.4 32.5" />
      {/* mast */}
      <path className="ln" d="M121.3 136C120.6 104 121.1 66 120.2 20.5" />
      {/* jib */}
      <path className="ln sail" d="M116.8 34.2C101.4 59.8 86.2 90.4 72.6 121.4C87.6 122.3 101.8 122.1 116.2 121.2C116.9 92.4 116.4 62.6 116.8 34.2Z" />
      {/* mainsail, with the logo's waves */}
      <path className="ln sail" d="M124.4 25.8C151.2 51.6 172.4 86.2 183.2 120.6C164.6 121.8 143.4 121.6 125.2 120.4C125.8 89.6 124.8 57.8 124.4 25.8Z" />
      <path className="ln" d="M133.2 84.6c4.1-3.4 7.7-3.6 11.2-.2s7.4 3.3 11.4-.3" />
      <path className="ln" d="M134 96.4c4.1-3.4 7.7-3.6 11.2-.2s7.4 3.3 11.4-.3" />
      <path className="ln" d="M134.6 108.2c4.1-3.4 7.7-3.6 11.2-.2s7.4 3.3 11.4-.3" />
      {/* crew, standing on deck behind the gunwale */}
      <Critter x={44} y={124.5} color="terracotta" kind="pirate" delay={0.4} />
      <Critter x={70} y={125.5} color="mustard" kind="bow" delay={2.1} />
      <Critter x={96} y={125} color="sage" kind="flag" delay={3.3} />
      <Critter x={146} y={125.2} color="plum" kind="headphones" delay={1.2} />
      <Critter x={172} y={124.6} color="blue" kind="spyglass" delay={0} />
      <Critter x={198} y={125.4} color="slate" kind="wave" delay={4.2} />
      {/* hull */}
      <path
        className="ln hull"
        d="M20.6 131.4C70.4 136.2 168.8 136.4 222.4 129.6C215.6 143.8 205.2 156.2 192.4 164.6C150.2 170.6 92.6 170.2 49.4 164.2C37.4 156.4 28.4 145.8 20.6 131.4Z"
      />
      <path className="ln faint" d="M23.4 130.6C72 134.6 168 135.2 219.8 128.8" />
      <path className="ln" d="M33.8 146.2C88.6 152.4 160.2 152.6 210.4 144.4" />
      <circle className="ln port" cx="84" cy="157.6" r="2.8" />
      <circle className="ln port" cx="120.4" cy="159" r="2.8" />
      <circle className="ln port" cx="156.6" cy="157.4" r="2.8" />
    </svg>
  )
}
