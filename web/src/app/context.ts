import { createContext, useContext } from 'react'
import type { Agent, Board, Me, TeamSummary } from '../api'

export interface ShellCtx {
  me: Me
  /** API id of the current org (`/api/teams/:tid`). */
  teamId: string
  /** URL slug of the current org. */
  slug: string
  /** Path prefix for in-app links: `/<slug>`. */
  base: string
  team: TeamSummary
  boards: Board[]
  agents: Agent[]
  unread: number
  refreshBoards: () => Promise<void>
  refreshMe: () => Promise<void>
  refreshUnread: () => Promise<void>
  openNav: () => void
}

export const ShellContext = createContext<ShellCtx | null>(null)

export function useShell(): ShellCtx {
  const ctx = useContext(ShellContext)
  if (!ctx) throw new Error('useShell outside the dashboard')
  return ctx
}
