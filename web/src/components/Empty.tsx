import type { ReactNode } from 'react'

/** A calm empty state: one line, an optional hint, at most one action. */
export function Empty({ title, hint, action, compact }: { title: ReactNode; hint?: ReactNode; action?: ReactNode; compact?: boolean }) {
  return (
    <div className={`empty ${compact ? 'compact' : ''}`}>
      <p className="empty-title">{title}</p>
      {hint ? <p className="empty-hint">{hint}</p> : null}
      {action ? <div className="empty-action">{action}</div> : null}
    </div>
  )
}
