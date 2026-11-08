import { createContext, useContext } from 'react'
import type { Agent, Board, Me, TeamSummary } from '../api'

export interface ShellCtx {
  me: Me
  teamId: string
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
