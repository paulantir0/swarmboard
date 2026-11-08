import { useRef, useState, type ClipboardEvent, type DragEvent, type KeyboardEvent, type ReactNode } from 'react'
import { api } from '../api'
import { Icon } from './Icon'

/** Markdown textarea with image paste/drop/attach (uploads, then inserts ![name](url)). Cmd/Ctrl+Enter submits. */
export function MarkdownField(props: {
  teamId: string
  value: string
  onChange: (v: string) => void
  onSubmit?: () => void
  placeholder?: string
  rows?: number
  autoFocus?: boolean
  label?: string
  id?: string
  /** Buttons rendered at the right end of the toolbar row. */
  actions?: ReactNode
}) {
  const { teamId, value, onChange, onSubmit, placeholder, rows = 4, autoFocus, label, id, actions } = props
  const ref = useRef<HTMLTextAreaElement>(null)
  const fileRef = useRef<HTMLInputElement>(null)
  const [uploading, setUploading] = useState(0)
  const [error, setError] = useState('')
  const [dragging, setDragging] = useState(false)
  const valueRef = useRef(value)
  valueRef.current = value

  const insert = (text: string) => {
    const el = ref.current
    const cur = valueRef.current
    const start = el?.selectionStart ?? cur.length
    const end = el?.selectionEnd ?? cur.length
    const before = cur.slice(0, start)
    const pad = before && !before.endsWith('\n') ? '\n' : ''
    const next = `${before}${pad}${text}\n${cur.slice(end)}`
    onChange(next)
    requestAnimationFrame(() => {
      if (!el) return
      const pos = before.length + pad.length + text.length + 1
      el.focus()
      el.setSelectionRange(pos, pos)
    })
  }

  const upload = async (files: File[]) => {
    const list = files.filter((f) => f.size > 0)
    if (!list.length) return
    setError('')
    for (const file of list) {
      if (file.size > 10 * 1024 * 1024) {
        setError('Files are limited to 10 MB')
        continue
      }
      setUploading((n) => n + 1)
      try {
        const info = await api.upload(teamId, file)
        const name = info.filename.replace(/[[\]]/g, '')
        insert(info.content_type.startsWith('image/') ? `![${name}](${info.url})` : `[${name}](${info.url})`)
      } catch (err) {
        setError(err instanceof Error ? err.message : 'Upload failed')
      } finally {
        setUploading((n) => n - 1)
      }
    }
  }

  const onPaste = (e: ClipboardEvent<HTMLTextAreaElement>) => {
    const files = [...e.clipboardData.files]
    if (files.length) {
      e.preventDefault()
      void upload(files)
    }
  }

  const onDrop = (e: DragEvent<HTMLDivElement>) => {
    setDragging(false)
    const files = [...e.dataTransfer.files]
    if (files.length) {
      e.preventDefault()
      void upload(files)
    }
  }

  const onKeyDown = (e: KeyboardEvent<HTMLTextAreaElement>) => {
    if (e.key === 'Enter' && (e.metaKey || e.ctrlKey) && onSubmit) {
      e.preventDefault()
      onSubmit()
    }
  }

  return (
    <div
      className={`md-field ${dragging ? 'dragging' : ''}`}
      onDragOver={(e) => {
        if (e.dataTransfer.types.includes('Files')) {
          e.preventDefault()
          setDragging(true)
        }
      }}
      onDragLeave={() => setDragging(false)}
      onDrop={onDrop}
    >
      {label ? (
        <label className="label" htmlFor={id}>
          {label}
        </label>
      ) : null}
      <textarea
        id={id}
        ref={ref}
        className="textarea"
        rows={rows}
        placeholder={placeholder}
        value={value}
        autoFocus={autoFocus}
        onChange={(e) => onChange(e.target.value)}
        onPaste={onPaste}
        onKeyDown={onKeyDown}
      />
      <div className="md-tools">
        <button type="button" className="btn btn-ghost btn-sm" onClick={() => fileRef.current?.click()} disabled={uploading > 0}>
          <Icon name="image" size={14} />
          {uploading > 0 ? 'Uploading' : 'Attach'}
        </button>
        <span className="md-hint muted">Markdown</span>
        {error ? <span className="error-text md-error">{error}</span> : null}
        {actions ? <span className="md-actions">{actions}</span> : null}
        <input
          ref={fileRef}
          type="file"
          hidden
          multiple
          onChange={(e) => {
            const files = [...(e.target.files ?? [])]
            e.target.value = ''
            void upload(files)
          }}
        />
      </div>
    </div>
  )
}
