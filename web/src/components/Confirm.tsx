import { useEffect, useRef, useState, type ReactNode } from 'react'

/** Inline "Question? [Yes] [Cancel]" in place of a native confirm(). Esc or Cancel backs out. */
export function Confirm({
  question,
  confirmLabel,
  onConfirm,
  onCancel,
  danger,
  busy,
  className = '',
}: {
  question: ReactNode
  confirmLabel: string
  onConfirm: () => void
  onCancel: () => void
  danger?: boolean
  busy?: boolean
  className?: string
}) {
  const cancelRef = useRef<HTMLButtonElement>(null)
  useEffect(() => {
    // Destructive by default: keyboard focus lands on Cancel.
    cancelRef.current?.focus()
  }, [])
  return (
    <span
      className={`confirm ${className}`}
      role="group"
      aria-label="Confirm"
      onKeyDown={(e) => {
        if (e.key === 'Escape') {
          e.stopPropagation()
          onCancel()
        }
      }}
    >
      <span className="confirm-q">{question}</span>
      <button type="button" className={`btn btn-sm ${danger ? 'btn-danger-solid' : 'btn-primary'}`} onClick={onConfirm} disabled={busy}>
        {confirmLabel}
      </button>
      <button type="button" ref={cancelRef} className="btn btn-sm btn-ghost" onClick={onCancel}>
        Cancel
      </button>
    </span>
  )
}

/** A button that asks inline before acting. */
export function ConfirmButton({
  children,
  question,
  confirmLabel,
  onConfirm,
  className = 'btn btn-sm',
  danger,
  disabled,
  title,
}: {
  children: ReactNode
  question: ReactNode
  confirmLabel: string
  onConfirm: () => void | Promise<unknown>
  className?: string
  danger?: boolean
  disabled?: boolean
  title?: string
}) {
  const [asking, setAsking] = useState(false)
  const [busy, setBusy] = useState(false)
  const triggerRef = useRef<HTMLButtonElement>(null)
  const refocus = useRef(false)

  useEffect(() => {
    if (!asking && refocus.current) {
      refocus.current = false
      triggerRef.current?.focus()
    }
  }, [asking])

  if (asking) {
    return (
      <Confirm
        question={question}
        confirmLabel={confirmLabel}
        danger={danger}
        busy={busy}
        onCancel={() => {
          refocus.current = true
          setAsking(false)
        }}
        onConfirm={async () => {
          setBusy(true)
          try {
            await onConfirm()
          } finally {
            setBusy(false)
            setAsking(false)
          }
        }}
      />
    )
  }
  return (
    <button type="button" ref={triggerRef} className={className} disabled={disabled} title={title} onClick={() => setAsking(true)}>
      {children}
    </button>
  )
}
