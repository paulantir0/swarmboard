import { useEffect, useState } from 'react'
import { copyText } from '../lib/clipboard'
import { Icon } from './Icon'

export function CopyButton({ text, label = 'Copy', className = '' }: { text: string; label?: string; className?: string }) {
  const [done, setDone] = useState(false)
  useEffect(() => {
    if (!done) return
    const id = window.setTimeout(() => setDone(false), 1400)
    return () => window.clearTimeout(id)
  }, [done])
  return (
    <button type="button" className={`btn btn-sm ${className}`} onClick={async () => setDone(await copyText(text))} aria-label={`${label}`}>
      <Icon name={done ? 'check' : 'copy'} size={12} />
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
