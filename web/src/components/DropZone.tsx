import { useEffect, useRef, useState, useSyncExternalStore } from 'react'
import { Icon } from './Icon'

/*
 * Whole-window file drop. Markdown fields that opt in (`dropTarget` priority) register here; while any
 * are mounted, dragging files over the window shows "Drop to attach" and a drop anywhere goes to the
 * field focused most recently, else the highest priority. A drop straight onto a field is handled by
 * that field. With no targets, file drops are swallowed so the browser doesn't navigate to the file.
 */

interface Target {
  priority: number
  focusedAt: number
  upload: (files: File[]) => void
}

const targets = new Map<number, Target>()
const listeners = new Set<() => void>()
let nextId = 0

const emit = () => {
  for (const fn of listeners) fn()
}

const store = {
  subscribe(fn: () => void) {
    listeners.add(fn)
    return () => listeners.delete(fn)
  },
  count: () => targets.size,
}

function pick(): Target | undefined {
  let best: Target | undefined
  for (const t of targets.values()) {
    if (!best) best = t
    else if (t.focusedAt !== best.focusedAt) best = t.focusedAt > best.focusedAt ? t : best
    else if (t.priority > best.priority) best = t
  }
  return best
}

/** Register a window-drop target. Returns a function to call when the field gains focus. */
export function useDropTarget(priority: number | undefined, upload: (files: File[]) => void): () => void {
  const uploadRef = useRef(upload)
  uploadRef.current = upload
  const idRef = useRef(0)

  useEffect(() => {
    if (priority === undefined) return
    const id = ++nextId
    idRef.current = id
    targets.set(id, { priority, focusedAt: 0, upload: (files) => uploadRef.current(files) })
    emit()
    return () => {
      targets.delete(id)
      idRef.current = 0
      emit()
    }
  }, [priority])

  return () => {
    const t = targets.get(idRef.current)
    if (t) t.focusedAt = performance.now()
  }
}

const hasFiles = (e: DragEvent) => Boolean(e.dataTransfer && Array.from(e.dataTransfer.types).includes('Files'))

/** Mounted once in the dashboard shell. */
export function WindowDrop() {
  const count = useSyncExternalStore(store.subscribe, store.count, store.count)
  const [over, setOver] = useState(false)
  const depth = useRef(0)
  const enabled = count > 0

  useEffect(() => {
    let watchdog = 0
    const reset = () => {
      window.clearTimeout(watchdog)
      depth.current = 0
      setOver(false)
    }
    const onEnter = (e: DragEvent) => {
      if (!hasFiles(e)) return
      depth.current++
      if (targets.size) setOver(true)
    }
    const onLeave = (e: DragEvent) => {
      if (!hasFiles(e)) return
      depth.current = Math.max(0, depth.current - 1)
      if (depth.current === 0) reset()
    }
    const onOver = (e: DragEvent) => {
      if (!hasFiles(e)) return
      e.preventDefault()
      if (e.dataTransfer) e.dataTransfer.dropEffect = targets.size ? 'copy' : 'none'
      if (targets.size) setOver(true)
      // dragover repeats while the pointer is over the page; when it stops, the drag has left.
      window.clearTimeout(watchdog)
      watchdog = window.setTimeout(reset, 400)
    }
    const onDrop = (e: DragEvent) => {
      if (!hasFiles(e)) return
      reset()
      if (e.defaultPrevented) return // a field took it
      e.preventDefault()
      const files = Array.from(e.dataTransfer?.files ?? [])
      if (files.length) pick()?.upload(files)
    }
    const onKey = (e: KeyboardEvent) => e.key === 'Escape' && reset()
    window.addEventListener('dragenter', onEnter)
    window.addEventListener('dragleave', onLeave)
    window.addEventListener('dragover', onOver)
    window.addEventListener('drop', onDrop)
    window.addEventListener('dragend', reset)
    window.addEventListener('keydown', onKey)
    return () => {
      window.clearTimeout(watchdog)
      window.removeEventListener('dragenter', onEnter)
      window.removeEventListener('dragleave', onLeave)
      window.removeEventListener('dragover', onOver)
      window.removeEventListener('drop', onDrop)
      window.removeEventListener('dragend', reset)
      window.removeEventListener('keydown', onKey)
    }
  }, [])

  if (!enabled || !over) return null
  return (
    <div className="drop-overlay" aria-hidden="true">
      <div className="drop-card">
        <Icon name="paperclip" size={18} />
        <span>Drop to attach</span>
      </div>
    </div>
  )
}
