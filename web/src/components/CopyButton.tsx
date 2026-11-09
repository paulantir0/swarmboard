import { useEffect, useState } from 'react'
import { copyText } from '../lib/clipboard'
import { Icon } from './Icon'

/**
 * size: xs (24, in snippet heads), sm (28, toolbars and rows), md (32, beside a 32px field).
 * name: what screen readers hear when the visible label alone is ambiguous ("Copy invite link").
 */
export function CopyButton({
  text,
  label = 'Copy',
  name,
  className = '',
  size = 'xs',
}: {
  text: string
  label?: string
  name?: string
  className?: string
  size?: 'xs' | 'sm' | 'md'
}) {
  const [state, setState] = useState<'idle' | 'done' | 'failed'>('idle')
  useEffect(() => {
    if (state === 'idle') return
    const id = window.setTimeout(() => setState('idle'), 1400)
    return () => window.clearTimeout(id)
  }, [state])
  const shown = state === 'done' ? 'Copied' : state === 'failed' ? 'Copy failed' : label
  return (
    <button
      type="button"
      className={`btn ${size === 'md' ? '' : `btn-${size}`} ${className}`}
      onClick={async () => setState((await copyText(text)) ? 'done' : 'failed')}
      aria-label={state === 'idle' && name ? name : undefined}
      aria-live="polite"
    >
      <Icon name={state === 'done' ? 'check' : 'copy'} size={size === 'xs' ? 12 : 14} />
      {shown}
    </button>
  )
}

export function Snippet({ title, hint, text }: { title: string; hint?: string; text: string }) {
  return (
    <div className="snippet">
      <div className="snippet-head">
        <span className="snippet-title">{title}</span>
        {hint ? <span className="snippet-hint mono muted">{hint}</span> : null}
        <span className="spacer" />
        <CopyButton text={text} name={`Copy ${title}`} />
      </div>
      <pre className="snippet-body">
        <code>{text}</code>
      </pre>
    </div>
  )
}
