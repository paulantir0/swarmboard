import {
  useEffect,
  useRef,
  useState,
  type ClipboardEvent,
  type DragEvent,
  type KeyboardEvent,
  type ReactNode,
  type RefObject,
} from 'react'
import { api, MAX_FILE_BYTES, type FileInfo } from '../api'
import { useDropTarget } from './DropZone'
import { Icon } from './Icon'

export interface UploadItem {
  id: number
  name: string
  image: boolean
  progress: number
  error?: string
}

export function formatBytes(n: number): string {
  if (n < 1024) return `${n} B`
  if (n < 1024 * 1024) return `${(n / 1024).toFixed(n < 10 * 1024 ? 1 : 0)} KB`
  return `${(n / 1024 / 1024).toFixed(1)} MB`
}

const isImage = (type: string) => type.startsWith('image/')
const cleanName = (name: string) => name.replace(/[[\]\n\r]/g, '').trim() || 'file'
const mdUrl = (url: string) => (/[\s()<>]/.test(url) ? `<${url.replace(/>/g, '%3E')}>` : url)

/** Markdown for an uploaded file: an image, or a link titled with its size. */
export function fileMarkdown(info: FileInfo): string {
  const name = cleanName(info.filename)
  return isImage(info.content_type) ? `![${name}](${mdUrl(info.url)})` : `[${name}](${mdUrl(info.url)} "${formatBytes(info.size)}")`
}

/**
 * Upload files into a text field: inserts an "Uploading…" placeholder at the cursor per file, swaps it for
 * markdown when the upload finishes (or drops it on failure), and tracks progress and errors.
 */
export function useUploads(opts: {
  teamId: string
  ref: RefObject<HTMLTextAreaElement | HTMLInputElement | null>
  value: string
  onChange: (v: string) => void
  /** Put each file on its own line (textarea) rather than inline (single-line input). */
  block?: boolean
}) {
  const { teamId, ref, onChange, block = true } = opts
  const valueRef = useRef(opts.value)
  valueRef.current = opts.value
  const onChangeRef = useRef(onChange)
  onChangeRef.current = onChange
  const [items, setItems] = useState<UploadItem[]>([])
  const seq = useRef(0)
  const alive = useRef(true)
  const aborts = useRef(new Set<AbortController>())

  useEffect(() => {
    alive.current = true
    const pending = aborts.current
    return () => {
      alive.current = false
      for (const c of pending) c.abort()
    }
  }, [])

  const commit = (next: string, caret?: number) => {
    valueRef.current = next
    onChangeRef.current(next)
    const el = ref.current
    if (el && caret !== undefined && document.activeElement === el) {
      requestAnimationFrame(() => el.setSelectionRange(caret, caret))
    }
  }

  const insert = (text: string) => {
    const el = ref.current
    const cur = valueRef.current
    // An unfocused field whose caret was never moved appends rather than prepends.
    const fresh = el && document.activeElement !== el && el.selectionStart === 0 && el.selectionEnd === 0
    const start = fresh ? cur.length : (el?.selectionStart ?? cur.length)
    const end = fresh ? cur.length : (el?.selectionEnd ?? cur.length)
    const before = cur.slice(0, start)
    const after = cur.slice(end)
    const sep = block ? '\n' : ' '
    const pre = before && !before.endsWith(sep) && !before.endsWith('\n') ? sep : ''
    const post = block ? (after.startsWith('\n') ? '' : '\n') : after && !after.startsWith(' ') ? ' ' : ''
    const next = `${before}${pre}${text}${post}${after}`
    const caret = before.length + pre.length + text.length + post.length
    valueRef.current = next
    onChangeRef.current(next)
    requestAnimationFrame(() => {
      if (!el) return
      el.focus()
      el.setSelectionRange(caret, caret)
    })
  }

  /** Replace `from` with `to`; keeps the caret where it was relative to the text. */
  const replace = (from: string, to: string) => {
    const cur = valueRef.current
    let target = from
    let i = -1
    if (!to) {
      for (const variant of [`${from}\n`, ` ${from}`, from]) {
        i = cur.indexOf(variant)
        if (i >= 0) {
          target = variant
          break
        }
      }
    } else i = cur.indexOf(from)
    if (i < 0) return // removed by the user: treat as cancelled
    const next = cur.slice(0, i) + to + cur.slice(i + target.length)
    const el = ref.current
    const caret = el?.selectionStart ?? undefined
    commit(next, caret === undefined ? undefined : caret > i ? Math.max(i, caret + to.length - target.length) : caret)
  }

  const upload = (files: File[]) => {
    const list = files.filter((f) => f.size > 0)
    if (!list.length) return
    const fresh: UploadItem[] = []
    const jobs: { item: UploadItem; file: File; placeholder: string }[] = []
    for (const file of list) {
      const id = ++seq.current
      const name = cleanName(file.name || 'file')
      const image = isImage(file.type)
      if (file.size > MAX_FILE_BYTES) {
        fresh.push({ id, name, image, progress: 0, error: `${formatBytes(file.size)}, over the 10 MB limit` })
        continue
      }
      const item = { id, name, image, progress: 0 }
      fresh.push(item)
      jobs.push({ item, file, placeholder: `${image ? '!' : ''}[Uploading ${name}…](#upload-${id})` })
    }
    // New batch: forget old errors.
    setItems((prev) => [...prev.filter((p) => !p.error), ...fresh])
    if (jobs.length) insert(jobs.map((j) => j.placeholder).join(block ? '\n' : ' '))

    const update = (id: number, patch: Partial<UploadItem> | null) => {
      if (!alive.current) return
      setItems((prev) => (patch === null ? prev.filter((p) => p.id !== id) : prev.map((p) => (p.id === id ? { ...p, ...patch } : p))))
    }

    for (const { item, file, placeholder } of jobs) {
      const ctrl = new AbortController()
      aborts.current.add(ctrl)
      api
        .upload(teamId, file, (f) => update(item.id, { progress: f }), ctrl.signal)
        .then((info) => {
          if (!alive.current) return
          replace(placeholder, fileMarkdown(info))
          update(item.id, null)
        })
        .catch((err: unknown) => {
          if (!alive.current) return
          replace(placeholder, '')
          update(item.id, { error: err instanceof Error ? err.message : 'Upload failed' })
        })
        .finally(() => aborts.current.delete(ctrl))
    }
  }

  /** Clipboard files upload, unless it's text that also carries a picture of itself (spreadsheets, rich text). */
  const onPaste = (e: ClipboardEvent<HTMLTextAreaElement | HTMLInputElement>) => {
    const files = [...e.clipboardData.files]
    if (!files.length) return
    const text = e.clipboardData.getData('text/plain')
    if (text && files.every((f) => isImage(f.type))) return
    e.preventDefault()
    upload(files)
  }

  const dismiss = (id: number) => setItems((prev) => prev.filter((p) => p.id !== id))
  const busy = items.some((i) => !i.error)

  return { items, busy, upload, onPaste, dismiss }
}

/** Progress and error chips under a field. */
export function UploadList({ items, onDismiss }: { items: UploadItem[]; onDismiss: (id: number) => void }) {
  if (!items.length) return null
  return (
    <ul className="uploads" aria-live="polite">
      {items.map((u) => (
        <li key={u.id} className={`upload ${u.error ? 'failed' : ''}`}>
          <Icon name={u.image ? 'image' : 'file'} size={13} />
          <span className="upload-name mono">{u.name}</span>
          {u.error ? (
            <>
              <span className="upload-error">{u.error}</span>
              <button type="button" className="btn btn-ghost btn-sm btn-icon" onClick={() => onDismiss(u.id)} aria-label="Dismiss">
                <Icon name="x" size={12} />
              </button>
            </>
          ) : (
            <>
              <span className="upload-bar" aria-hidden="true">
                <span style={{ transform: `scaleX(${Math.max(0.04, u.progress)})` }} />
              </span>
              <span className="upload-pct mono">{Math.round(u.progress * 100)}%</span>
            </>
          )}
        </li>
      ))}
    </ul>
  )
}

/** Markdown textarea with file paste/drop/attach (uploads, then inserts markdown at the cursor). Cmd/Ctrl+Enter submits. */
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
  /** Receive whole-window file drops; higher wins when nothing was focused. */
  dropTarget?: number
  /** Called when uploads start or finish, so the parent can hold submit. */
  onBusyChange?: (busy: boolean) => void
}) {
  const { teamId, value, onChange, onSubmit, placeholder, rows = 4, autoFocus, label, id, actions, dropTarget, onBusyChange } = props
  const ref = useRef<HTMLTextAreaElement>(null)
  const fileRef = useRef<HTMLInputElement>(null)
  const [dragging, setDragging] = useState(false)
  const uploads = useUploads({ teamId, ref, value, onChange })
  const markFocused = useDropTarget(dropTarget, uploads.upload)

  const busyRef = useRef(onBusyChange)
  busyRef.current = onBusyChange
  useEffect(() => busyRef.current?.(uploads.busy), [uploads.busy])

  const onDrop = (e: DragEvent<HTMLDivElement>) => {
    setDragging(false)
    const files = [...e.dataTransfer.files]
    if (files.length) {
      e.preventDefault()
      uploads.upload(files)
    }
  }

  const onKeyDown = (e: KeyboardEvent<HTMLTextAreaElement>) => {
    if (e.key === 'Enter' && (e.metaKey || e.ctrlKey) && onSubmit) {
      e.preventDefault()
      if (!uploads.busy) onSubmit()
    }
  }

  const pending = uploads.items.filter((i) => !i.error).length

  return (
    <div
      className={`md-field ${dragging ? 'dragging' : ''}`}
      onDragOver={(e) => {
        if (e.dataTransfer.types.includes('Files')) {
          e.preventDefault()
          e.dataTransfer.dropEffect = 'copy'
          setDragging(true)
        }
      }}
      onDragLeave={(e) => {
        if (!e.currentTarget.contains(e.relatedTarget as Node | null)) setDragging(false)
      }}
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
        onPaste={uploads.onPaste}
        onKeyDown={onKeyDown}
        onFocus={markFocused}
      />
      <UploadList items={uploads.items} onDismiss={uploads.dismiss} />
      <div className="md-tools">
        <button type="button" className="btn btn-ghost btn-sm" onClick={() => fileRef.current?.click()} title="Attach files (or drop, or paste)">
          <Icon name="paperclip" size={14} />
          {pending ? `Uploading ${pending}` : 'Attach'}
        </button>
        <span className="md-hint muted">Markdown</span>
        {actions ? <span className="md-actions">{actions}</span> : null}
        <input
          ref={fileRef}
          type="file"
          hidden
          multiple
          onChange={(e) => {
            const files = [...(e.target.files ?? [])]
            e.target.value = ''
            uploads.upload(files)
          }}
        />
      </div>
    </div>
  )
}
