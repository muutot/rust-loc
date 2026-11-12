// The live progress stream: one EventSource, automatic reconnect, every
// snapshot handed to the store. The token rides in the query because an
// EventSource cannot set headers.

import type { StatusSnapshot } from './types'
import { token } from './api'

export function openEvents(onSnapshot: (snapshot: StatusSnapshot) => void): () => void {
  let source: EventSource | null = null
  let retry: ReturnType<typeof setTimeout> | null = null
  let closed = false
  let delay = 1500

  const connect = () => {
    if (closed) return
    source = new EventSource(`/api/events?token=${token()}`)
    source.onmessage = (message) => {
      delay = 1500
      try {
        onSnapshot(JSON.parse(message.data) as StatusSnapshot)
      } catch {
        // A malformed frame is skipped, the next one carries the truth.
      }
    }
    source.onerror = () => {
      source?.close()
      source = null
      if (!closed) {
        // Back off instead of hammering a dead or unauthorized server.
        retry = setTimeout(connect, delay)
        delay = Math.min(delay * 2, 30000)
      }
    }
  }
  connect()

  return () => {
    closed = true
    if (retry) clearTimeout(retry)
    source?.close()
  }
}
