import type { ReactNode, SelectHTMLAttributes } from 'react'
import { Icon } from './Icon'

/**
 * A native <select> with our own chevron and an optional leading glyph.
 * - pill: a 28px toolbar filter; `on` marks it as narrowing the view.
 * - quiet: borderless until hover, for property panels.
 */
export function Select({
  variant = 'pill',
  on,
  icon,
  className = '',
  children,
  ...rest
}: SelectHTMLAttributes<HTMLSelectElement> & { variant?: 'pill' | 'quiet'; on?: boolean; icon?: ReactNode }) {
  return (
    <span className={`select-wrap ${variant} ${on ? 'on' : ''} ${icon ? 'has-icon' : ''} ${className}`}>
      {icon ? <span className="select-icon">{icon}</span> : null}
      <select className="select-native" {...rest}>
        {children}
      </select>
      <Icon name="chevronDown" size={14} className="select-chev" />
    </span>
  )
}
