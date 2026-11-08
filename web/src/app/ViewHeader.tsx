import type { ReactNode } from 'react'
import { Icon } from '../components/Icon'
import { useShell } from './context'

export function ViewHeader({ title, children, back }: { title: ReactNode; children?: ReactNode; back?: ReactNode }) {
  const { openNav } = useShell()
  return (
    <header className="view-header">
      <button className="btn btn-ghost btn-icon nav-toggle" onClick={openNav} aria-label="Menu">
        <Icon name="menu" />
      </button>
      {back}
      <h1 className="view-title">{title}</h1>
      {children ? <div className="view-actions">{children}</div> : null}
    </header>
  )
}
