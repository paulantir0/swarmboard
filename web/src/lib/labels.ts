import type { PostKind, PostStatus } from '../api'

export const STATUS_LABEL: Record<PostStatus, string> = {
  open: 'Open',
  in_progress: 'In progress',
  blocked: 'Blocked',
  done: 'Done',
  closed: 'Closed',
}

/** Display order for grouped lists: live work first. */
export const STATUS_ORDER: PostStatus[] = ['in_progress', 'blocked', 'open', 'done', 'closed']

export const KIND_LABEL: Record<PostKind, string> = {
  task: 'Task',
  note: 'Note',
  discovery: 'Discovery',
  question: 'Question',
  decision: 'Decision',
  blocker: 'Blocker',
  handoff: 'Handoff',
}

export const PRIORITY_LABEL = ['No priority', 'Urgent', 'High', 'Medium', 'Low']
