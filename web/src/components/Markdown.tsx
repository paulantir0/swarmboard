import { useEffect, useMemo, useRef, useState, type MouseEvent } from 'react'
import { renderMarkdown } from '../lib/markdown'
import { Icon } from './Icon'

/** Uploaded files live at /f/<id>/<name> on the API host. */
const FILE_PATH = /^\/f\/[A-Za-z0-9_-]+\/[^/]+$/
const SIZE = /^\d+(\.\d+)? (B|KB|MB)$/
const SAFE_SRC = /^(https?:)?\//i

const FILE_ICON =
  '<svg class="md-file-icon" width="13" height="13" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round" aria-hidden="true"><path d="M15 2H6a2 2 0 0 0-2 2v16a2 2 0 0 0 2 2h12a2 2 0 0 0 2-2V7Z"/><path d="M14 2v4a2 2 0 0 0 2 2h4"/></svg>'

/** Sanitized HTML → images become click-to-open thumbnails; links to uploads get a file icon and size. */
function enhance(html: string): string {
  if (!html.includes('<img') && !html.includes('/f/')) return html
  const tpl = document.createElement('template')
  tpl.innerHTML = html
  const root = tpl.content
  for (const img of root.querySelectorAll('img')) {
    const src = img.getAttribute('src') ?? ''
    img.classList.add('md-thumb')
    if (img.closest('a') || !SAFE_SRC.test(src)) continue
    const a = document.createElement('a')
    a.href = src
    a.className = 'md-thumb-link'
    a.target = '_blank'
    a.rel = 'noopener noreferrer'
    a.title = img.getAttribute('alt') || ''
    img.replaceWith(a)
    a.appendChild(img)
  }
  for (const a of root.querySelectorAll('a[href]')) {
    if (a.classList.contains('md-thumb-link') || a.querySelector('img')) continue
    let path = ''
    try {
      path = new URL(a.getAttribute('href') ?? '', window.location.origin).pathname
    } catch {
      continue
    }
    if (!FILE_PATH.test(path)) continue
    a.classList.add('md-file')
    a.insertAdjacentHTML('afterbegin', FILE_ICON)
    const title = a.getAttribute('title') ?? ''
    if (SIZE.test(title)) {
      const size = document.createElement('span')
      size.className = 'md-file-size'
      size.textContent = title
      a.appendChild(size)
      a.removeAttribute('title')
    }
  }
  return tpl.innerHTML
}

export function Markdown({ source, className }: { source: string; className?: string }) {
  const html = useMemo(() => enhance(renderMarkdown(source)), [source])
  const [open, setOpen] = useState<{ src: string; alt: string } | null>(null)

  const onClick = (e: MouseEvent<HTMLDivElement>) => {
    if (e.button !== 0 || e.metaKey || e.ctrlKey || e.shiftKey || e.altKey) return
    const link = (e.target as HTMLElement).closest('a.md-thumb-link')
    const img = link?.querySelector('img')
    if (!link || !img) return
    e.preventDefault()
    setOpen({ src: link.getAttribute('href') ?? img.src, alt: img.alt })
  }

  return (
    <>
      <div className={`md ${className ?? ''}`} onClick={onClick} dangerouslySetInnerHTML={{ __html: html }} />
      {open ? <Lightbox src={open.src} alt={open.alt} onClose={() => setOpen(null)} /> : null}
    </>
  )
}

function Lightbox({ src, alt, onClose }: { src: string; alt: string; onClose: () => void }) {
  const ref = useRef<HTMLDialogElement>(null)
  // No close() on cleanup: it fires `close` (→ onClose) asynchronously, which under StrictMode's
  // mount/unmount/mount closes the box we just opened. Unmounting removes the modal anyway.
  useEffect(() => {
    const d = ref.current
    if (d && !d.open) d.showModal()
  }, [])
  return (
    <dialog
      ref={ref}
      className="lightbox"
      aria-label={alt || 'Image'}
      onClose={onClose}
      onClick={(e) => {
        if (e.target === e.currentTarget || (e.target as HTMLElement).tagName === 'IMG') onClose()
      }}
    >
      <img src={src} alt={alt} />
      <div className="lightbox-bar">
        <span className="lightbox-name mono">{alt}</span>
        <a className="btn btn-sm" href={src} target="_blank" rel="noopener noreferrer">
          <Icon name="externalLink" size={12} />
          Open
        </a>
        <button className="btn btn-ghost btn-sm btn-icon" onClick={onClose} aria-label="Close" autoFocus>
          <Icon name="x" size={12} />
        </button>
      </div>
    </dialog>
  )
}
