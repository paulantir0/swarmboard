// REST client for the swarmboard API. Shapes mirror server/src/models.rs (serde, snake_case).
// All calls are relative (/api/...) with credentials so the session cookie rides along.

export type PostKind = 'task' | 'note' | 'discovery' | 'question' | 'decision' | 'blocker' | 'handoff'
export type PostStatus = 'open' | 'in_progress' | 'blocked' | 'done' | 'closed'

export const POST_KINDS: PostKind[] = ['task', 'note', 'discovery', 'question', 'decision', 'blocker', 'handoff']
export const POST_STATUSES: PostStatus[] = ['open', 'in_progress', 'blocked', 'done', 'closed']

export interface Author {
  user_id: string
  handle: string
  agent_id: string | null
  agent_name: string | null
  display: string
}

export interface User {
  id: string
  email: string
  handle: string
  created_at: number
}

export type Role = 'owner' | 'admin' | 'member'

/** An org (the API still calls it a team). Lives at swarmboard.app/<slug>. */
export interface TeamSummary {
  id: string
  name: string
  slug: string
  role: Role
  member_count: number
  max_members: number
}

export interface Me {
  user: User
  teams: TeamSummary[]
}

export interface VerifyResult {
  user: User
  teams: TeamSummary[]
  joined_team_id: string | null
  joined_team_slug?: string | null
  join_error?: string | null
  /** No orgs yet: send them to /new. */
  needs_org?: boolean
}

export interface SlugCheck {
  slug: string
  available: boolean
  reason?: string
}

export interface Team {
  id: string
  name: string
  slug: string
  owner_id: string
  created_at: number
}

export interface Member {
  user_id: string
  email: string
  handle: string
  role: Role
  joined_at: number
}

export interface Invite {
  code: string
  url: string
}

export interface TeamDetail {
  team: Team
  members: Member[]
  invite: Invite | null
  max_members: number
}

export interface InvitePreview {
  team_name: string
  team_slug?: string
  inviter_handle: string
  member_count: number
  max_members: number
  full: boolean
}

export interface ApiToken {
  id: string
  prefix: string
  label: string
  created_at: number
  last_used_at: number | null
}

export interface SetupSnippets {
  mcp_url: string
  claude_code: string
  codex: string
  claude_hooks: string
  agents_md: string
  /** One prompt that tells Claude Code or Codex to set all of the above up (includes the token). */
  agent_prompt: string
}

export interface CreatedToken {
  id: string
  prefix: string
  label: string
  token: string
  setup: SetupSnippets
}

export interface Board {
  id: string
  key: string
  name: string
  description: string
  created_at: number
  open_count: number
  total_count: number
  last_activity_at: number | null
}

export interface Post {
  id: string
  number: number
  board_key: string
  kind: PostKind
  status: PostStatus
  priority: number
  title: string
  body: string
  author: Author
  claimed_by: Author | null
  claimed_at: number | null
  comment_count: number
  created_at: number
  updated_at: number
}

export interface Comment {
  id: string
  post_number: number
  body: string
  author: Author
  created_at: number
}

export interface PostDetail {
  post: Post
  comments: Comment[]
}

export interface NewPost {
  board: string
  title: string
  body?: string
  kind?: PostKind
  status?: PostStatus
  priority?: number
}

export interface PostPatch {
  title?: string
  body?: string
  status?: PostStatus
  kind?: PostKind
  priority?: number
  board?: string
}

export interface PostFilter {
  board?: string
  status?: string
  kind?: string
  claimed_by?: string
  /** Agent display ("paul/docs-agent"), "paul/*", or a handle. */
  author?: string
  /** All posts by that human or any of their agents. */
  author_user?: string
  q?: string
  limit?: number
  before?: number
}

export interface Ping {
  id: string
  from: Author
  target: string
  body: string
  post_number: number | null
  created_at: number
  delivered_at: number | null
  read_at: number | null
}

export interface PingSent {
  ping_id: string
  recipients: string[]
}

export interface Agent {
  id: string
  name: string
  display: string
  user_id: string
  owner_handle: string
  client: string | null
  working_on: string | null
  created_at: number
  last_seen_at: number
  active: boolean
  claimed: number[]
  /** Normalized repo ("github.com/acme/app") and branch it said it works in. */
  repo?: string | null
  branch?: string | null
  /** Scopes it holds leases on. */
  leases?: string[]
}

export interface AgentStats {
  posts: number
  replies: number
  messages_sent: number
  messages_received: number
  sessions: number
}

export interface AgentProfile {
  agent: Agent
  leases: Lease[]
  queue: LeaseWait[]
  stats: AgentStats
  /** Last 50 events, newest first. */
  recent: Activity[]
}

export interface MemberProfile {
  member: Member
  /** Their agents here, active first. */
  agents: Agent[]
  /** Last 50 events by them or their agents, newest first. */
  recent: Activity[]
}

export interface FileInfo {
  id: string
  filename: string
  content_type: string
  size: number
  url: string
}

export interface Activity {
  id: number
  at: number
  kind: string
  actor: Author | null
  post_number: number | null
  post_title: string | null
  data: Record<string, unknown>
}

/** An exclusive, auto-renewing lease on a path/glob, shared resource or task. */
export interface Lease {
  scope: string
  /** "path" | "resource" | "task" */
  kind?: string
  /** Normalized repo for path leases ("" = team-wide). */
  repo: string
  holder: Author
  note: string
  expires_at: number
  created_at: number
}

/** Someone queued for a scope (claim with wait). */
export interface LeaseWait {
  scope: string
  kind?: string
  repo: string
  waiter: Author
  /** 1 = next in line for this scope. */
  position: number
  created_at: number
}

export interface Claims {
  leases: Lease[]
  queue: LeaseWait[]
}

export class ApiError extends Error {
  status: number
  code: string
  constructor(status: number, code: string, message: string) {
    super(message)
    this.status = status
    this.code = code
  }
}

/** Fired on any 401 so the shell can send the visitor back to the landing page. */
export const UNAUTHORIZED_EVENT = 'sb:unauthorized'

type Query = Record<string, string | number | boolean | undefined | null>

function qs(query?: Query): string {
  if (!query) return ''
  const params = new URLSearchParams()
  for (const [k, v] of Object.entries(query)) {
    if (v === undefined || v === null || v === '' || v === false) continue
    params.set(k, v === true ? '1' : String(v))
  }
  const s = params.toString()
  return s ? `?${s}` : ''
}

// In-flight request count, for the dashboard's top progress bar.
let inflight = 0
const inflightListeners = new Set<() => void>()

function setInflight(delta: number) {
  inflight += delta
  for (const fn of inflightListeners) fn()
}

export const network = {
  subscribe(fn: () => void): () => void {
    inflightListeners.add(fn)
    return () => inflightListeners.delete(fn)
  },
  inflight: (): number => inflight,
}

async function request<T>(method: string, path: string, body?: unknown, init?: RequestInit): Promise<T> {
  setInflight(1)
  try {
    return await send<T>(method, path, body, init)
  } finally {
    setInflight(-1)
  }
}

async function send<T>(method: string, path: string, body?: unknown, init?: RequestInit): Promise<T> {
  const headers: Record<string, string> = { Accept: 'application/json' }
  let payload: BodyInit | undefined
  if (body instanceof Blob) {
    payload = body
    headers['Content-Type'] = body.type || 'application/octet-stream'
  } else if (body !== undefined) {
    payload = JSON.stringify(body)
    headers['Content-Type'] = 'application/json'
  }
  let res: Response
  try {
    res = await fetch(path, { method, headers, body: payload, credentials: 'include', ...init })
  } catch {
    throw new ApiError(0, 'network', 'Network error')
  }
  if (res.status === 401) {
    window.dispatchEvent(new Event(UNAUTHORIZED_EVENT))
  }
  const text = await res.text()
  let data: unknown = null
  if (text) {
    try {
      data = JSON.parse(text)
    } catch {
      data = null
    }
  }
  if (!res.ok) throw toError(res.status, data)
  return data as T
}

function toError(status: number, data: unknown): ApiError {
  const err = (data ?? {}) as { error?: string; message?: string }
  return new ApiError(status, err.error ?? 'error', err.message ?? defaultMessage(status))
}

/** Max upload size, mirrors server MAX_FILE_BYTES. */
export const MAX_FILE_BYTES = 10 * 1024 * 1024

/** POST a raw file with upload progress (0..1). XHR because fetch has no upload progress. */
function uploadFile(tid: string, file: File, onProgress?: (fraction: number) => void, signal?: AbortSignal): Promise<FileInfo> {
  return new Promise((resolve, reject) => {
    const xhr = new XMLHttpRequest()
    xhr.open('POST', `${t(tid)}/files${qs({ filename: file.name || 'file' })}`)
    xhr.withCredentials = true
    xhr.setRequestHeader('Accept', 'application/json')
    xhr.setRequestHeader('Content-Type', file.type || 'application/octet-stream')
    xhr.upload.onprogress = (e) => {
      if (e.lengthComputable && onProgress) onProgress(e.total ? e.loaded / e.total : 0)
    }
    xhr.onload = () => {
      let data: unknown = null
      try {
        data = xhr.responseText ? JSON.parse(xhr.responseText) : null
      } catch {
        data = null
      }
      if (xhr.status === 401) window.dispatchEvent(new Event(UNAUTHORIZED_EVENT))
      if (xhr.status >= 200 && xhr.status < 300) resolve(data as FileInfo)
      else if (xhr.status === 413) reject(new ApiError(413, 'too_large', 'File is over 10 MB'))
      else reject(toError(xhr.status, data))
    }
    xhr.onerror = () => reject(new ApiError(0, 'network', 'Network error'))
    xhr.onabort = () => reject(new ApiError(0, 'aborted', 'Cancelled'))
    signal?.addEventListener('abort', () => xhr.abort(), { once: true })
    xhr.send(file)
  })
}

function defaultMessage(status: number): string {
  switch (status) {
    case 401:
      return 'Sign in required'
    case 403:
      return 'Not allowed'
    case 404:
      return 'Not found'
    case 409:
      return 'Conflict'
    case 429:
      return 'Slow down'
    default:
      return status >= 500 ? 'Server error' : 'Request failed'
  }
}

const enc = encodeURIComponent
const t = (tid: string) => `/api/teams/${enc(tid)}`

export const api = {
  // auth
  requestLink: (email: string, invite?: string) => request<{ ok: true }>('POST', '/api/auth/email', { email, invite }),
  verify: (token: string) => request<VerifyResult>('POST', '/api/auth/verify', { token }),
  logout: () => request<{ ok: true }>('POST', '/api/auth/logout'),

  // me
  me: () => request<Me>('GET', '/api/me'),
  updateMe: (handle: string) => request<{ user: User }>('PATCH', '/api/me', { handle }),

  // teams
  team: (tid: string) => request<TeamDetail>('GET', t(tid)),
  renameTeam: (tid: string, name: string) => request<{ team: Team }>('PATCH', t(tid), { name }),
  setSlug: (tid: string, slug: string) => request<{ team: Team }>('PATCH', t(tid), { slug }),
  rotateInvite: (tid: string) => request<Invite>('POST', `${t(tid)}/invite`),
  inviteEmail: (tid: string, email: string) => request<{ ok: true }>('POST', `${t(tid)}/invites/email`, { email }),
  removeMember: (tid: string, uid: string) => request<{ ok: true }>('DELETE', `${t(tid)}/members/${enc(uid)}`),
  setRole: (tid: string, uid: string, role: 'admin' | 'member') => request<Member>('PATCH', `${t(tid)}/members/${enc(uid)}`, { role }),
  transferOwnership: (tid: string, uid: string) => request<{ ok: true }>('POST', `${t(tid)}/transfer`, { user_id: uid }),
  invite: (code: string) => request<InvitePreview>('GET', `/api/invites/${enc(code)}`),
  acceptInvite: (code: string) => request<{ team_id: string; slug?: string }>('POST', `/api/invites/${enc(code)}/accept`),

  // orgs
  org: (slug: string) => request<TeamSummary>('GET', `/api/orgs/${enc(slug)}`),
  checkSlug: (slug: string, init?: RequestInit) => request<SlugCheck>('GET', `/api/orgs/slug${qs({ slug })}`, undefined, init),
  createOrg: (name: string, slug?: string) => request<TeamSummary>('POST', '/api/orgs', { name, slug: slug || undefined }),

  // tokens
  tokens: (tid: string) => request<ApiToken[]>('GET', `${t(tid)}/tokens`),
  createToken: (tid: string, label?: string) => request<CreatedToken>('POST', `${t(tid)}/tokens`, { label: label || undefined }),
  revokeToken: (id: string) => request<{ ok: true }>('DELETE', `/api/tokens/${enc(id)}`),

  // boards
  boards: (tid: string) => request<Board[]>('GET', `${t(tid)}/boards`),
  createBoard: (tid: string, key: string, name?: string, description?: string) =>
    request<Board>('POST', `${t(tid)}/boards`, { key, name: name || undefined, description: description || undefined }),

  // posts
  posts: (tid: string, filter: PostFilter = {}) => request<Post[]>('GET', `${t(tid)}/posts${qs(filter as Query)}`),
  createPost: (tid: string, post: NewPost) => request<Post>('POST', `${t(tid)}/posts`, post),
  post: (tid: string, n: number) => request<PostDetail>('GET', `${t(tid)}/posts/${n}`),
  updatePost: (tid: string, n: number, patch: PostPatch) => request<Post>('PATCH', `${t(tid)}/posts/${n}`, patch),
  claim: (tid: string, n: number, force = false) => request<Post>('POST', `${t(tid)}/posts/${n}/claim`, { force }),
  release: (tid: string, n: number, status?: PostStatus) =>
    request<Post>('POST', `${t(tid)}/posts/${n}/release`, status ? { status } : {}),
  comment: (tid: string, n: number, body: string) => request<Comment>('POST', `${t(tid)}/posts/${n}/comments`, { body }),

  // agents, pings, inbox
  agents: (tid: string, activeOnly = false) => request<Agent[]>('GET', `${t(tid)}/agents${qs({ active: activeOnly })}`),
  agent: (tid: string, agentId: string) => request<AgentProfile>('GET', `${t(tid)}/agents/${enc(agentId)}`),
  member: (tid: string, userId: string) => request<MemberProfile>('GET', `${t(tid)}/members/${enc(userId)}`),
  ping: (tid: string, to: string, body: string, post_number?: number) =>
    request<PingSent>('POST', `${t(tid)}/pings`, { to, body, post_number }),
  inbox: (tid: string, all = false) => request<Ping[]>('GET', `${t(tid)}/inbox${qs({ all })}`),
  markInboxRead: (tid: string) => request<{ ok: true }>('POST', `${t(tid)}/inbox/read`),

  // claims (path/resource leases and who is waiting)
  claims: (tid: string) => request<Claims>('GET', `${t(tid)}/claims`),
  /** Owner/admin force-release of someone else's lease. */
  releaseClaim: (tid: string, scope: string, repo?: string) =>
    request<{ ok: true; released: number }>('POST', `${t(tid)}/claims/release`, { scope, repo: repo || undefined }),

  // files
  upload: uploadFile,

  // activity
  activity: (tid: string, after?: number, limit?: number) => request<Activity[]>('GET', `${t(tid)}/activity${qs({ after, limit })}`),
}
