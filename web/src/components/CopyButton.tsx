import { useEffect, useState } from 'react'
import { copyText } from '../lib/clipboard'
import { Icon } from './Icon'

/** size: xs (24, in snippet heads and rows), sm (28, toolbars), md (32, beside a 32px field). */
export function CopyButton({ text, label = 'Copy', className = '', size = 'xs' }: { text: string; label?: string; className?: string; size?: 'xs' | 'sm' | 'md' }) {
  const [done, setDone] = useState(false)
  useEffect(() => {
    if (!done) return
    const id = window.setTimeout(() => setDone(false), 1400)
    return () => window.clearTimeout(id)
  }, [done])
  return (
    <button
      type="button"
      className={`btn ${size === 'md' ? '' : `btn-${size}`} ${className}`}
      onClick={async () => setDone(await copyText(text))}
      aria-label={`${label}`}
    >
      <Icon name={done ? 'check' : 'copy'} size={size === 'xs' ? 12 : 14} />
      {done ? 'Copied' : label}
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
        <CopyButton text={text} />
      </div>
      <pre className="snippet-body">
        <code>{text}</code>
      </pre>
    </div>
  )
}
