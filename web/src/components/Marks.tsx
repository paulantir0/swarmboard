import type { Author, PostStatus } from '../api'
import { PRIORITY_LABEL, STATUS_LABEL } from '../lib/labels'

/** Linear-style status glyph, 14px. Color carries meaning only. */
export function StatusIcon({ status, size = 14 }: { status: PostStatus; size?: number }) {
  const label = STATUS_LABEL[status]
  const common = { width: size, height: size, viewBox: '0 0 14 14', role: 'img' as const, 'aria-label': label }
  switch (status) {
    case 'open':
      return (
        <svg {...common} className="status-icon s-open">
          <circle cx="7" cy="7" r="5.5" fill="none" stroke="currentColor" strokeWidth="1.5" />
        </svg>
      )
    case 'in_progress':
      return (
        <svg {...common} className="status-icon s-progress">
          <circle cx="7" cy="7" r="5.5" fill="none" stroke="currentColor" strokeWidth="1.5" />
          <path d="M7 3.5a3.5 3.5 0 0 1 0 7z" fill="currentColor" />
        </svg>
      )
    case 'blocked':
      return (
        <svg {...common} className="status-icon s-blocked">
          <circle cx="7" cy="7" r="5.5" fill="none" stroke="currentColor" strokeWidth="1.5" />
          <path d="M4.5 7h5" stroke="currentColor" strokeWidth="1.5" strokeLinecap="round" />
        </svg>
      )
    case 'done':
      return (
        <svg {...common} className="status-icon s-done">
          <circle cx="7" cy="7" r="6.25" fill="currentColor" />
          <path d="m4.5 7.2 1.8 1.8 3.4-3.6" fill="none" stroke="var(--color-canvas)" strokeWidth="1.5" strokeLinecap="round" strokeLinejoin="round" />
        </svg>
      )
    case 'closed':
      return (
        <svg {...common} className="status-icon s-closed">
          <circle cx="7" cy="7" r="6.25" fill="currentColor" />
          <path d="m5 5 4 4M9 5 5 9" stroke="var(--color-canvas)" strokeWidth="1.5" strokeLinecap="round" />
        </svg>
      )
  }
}

/** 0 none, 1 urgent, 2 high, 3 medium, 4 low. */
export function PriorityIcon({ priority, size = 14 }: { priority: number; size?: number }) {
  const label = PRIORITY_LABEL[priority] ?? PRIORITY_LABEL[0]
  if (priority === 1) {
    return (
      <svg width={size} height={size} viewBox="0 0 14 14" role="img" aria-label={label} className="priority-icon p-urgent">
        <rect x="1" y="1" width="12" height="12" rx="3" fill="currentColor" />
        <path d="M7 4v3.5" stroke="var(--color-canvas)" strokeWidth="1.6" strokeLinecap="round" />
        <circle cx="7" cy="10" r="0.9" fill="var(--color-canvas)" />
      </svg>
    )
  }
  const filled = priority === 2 ? 3 : priority === 3 ? 2 : priority === 4 ? 1 : 0
  return (
    <svg width={size} height={size} viewBox="0 0 14 14" role="img" aria-label={label} className="priority-icon">
      {[0, 1, 2].map((i) =>
        priority === 0 ? (
          <rect key={i} x={1.5 + i * 4} y="6.25" width="3" height="1.5" rx="0.75" fill="currentColor" opacity="0.5" />
        ) : (
          <rect
            key={i}
            x={1.5 + i * 4}
            y={9 - i * 3}
            width="3"
            height={3 + i * 3}
            rx="0.75"
            fill="currentColor"
            opacity={i < filled ? 1 : 0.25}
          />
        ),
      )}
    </svg>
  )
}

/** Identity, always showing whose agent it is: handle in muted, /agent in ink. */
export function Who({ author, me }: { author: Author; me?: string }) {
  const isMe = me !== undefined && author.user_id === me && !author.agent_id
  if (!author.agent_name) {
    return (
      <span className="who" title={author.display}>
        <span className="who-handle-solo">{isMe ? 'you' : author.handle}</span>
      </span>
    )
  }
  return (
    <span className="who" title={author.display}>
      <span className="who-handle">{author.handle}/</span>
      <span className="who-agent">{author.agent_name}</span>
    </span>
  )
}
