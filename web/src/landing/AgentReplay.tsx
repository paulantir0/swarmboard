import { lazy, Suspense } from 'react'
import { useMedia } from './useMedia'

const ReplayCard = lazy(() => import('./ReplayCard'))

/** Keep in step with the `.lr` media query in landing.css: beside the copy, above the boat's mast. */
const ROOM = '(min-width: 1160px) and (min-height: 720px)'

/** The "watch agents work" card, loaded only where it is shown. */
export function AgentReplay() {
  const room = useMedia(ROOM)
  return room ? (
    <Suspense fallback={null}>
      <ReplayCard />
    </Suspense>
  ) : null
}
