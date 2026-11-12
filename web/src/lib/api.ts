// Every call carries the per-run token: from the header a fetch can set, or
// the query an EventSource needs. The token arrives in the page URL the tray
// opens and moves into sessionStorage so a reload keeps working.

import type {
  Authors,
  Config,
  DirListing,
  LangInfo,
  Report,
  StatusSnapshot,
} from './types'

const KEY = 'rust-loc-token'

export function captureToken(): void {
  const fromUrl = new URLSearchParams(window.location.search).get('token')
  if (fromUrl) {
    sessionStorage.setItem(KEY, fromUrl)
    // Keep the address bar clean; the token survives in the session.
    window.history.replaceState({}, '', window.location.pathname)
  }
}

export function token(): string {
  return sessionStorage.getItem(KEY) ?? ''
}

async function request<T>(path: string, init?: RequestInit): Promise<T> {
  const response = await fetch(path, {
    ...init,
    headers: { 'x-auth-token': token(), ...(init?.headers ?? {}) },
  })
  if (!response.ok) {
    const text = await response.text()
    throw new Error(text || `${response.status}`)
  }
  return response.json() as Promise<T>
}

export const api = {
  config: () => request<Config>('/api/config'),
  saveConfig: (config: Config) =>
    request<Config>('/api/config', {
      method: 'PUT',
      headers: { 'content-type': 'application/json' },
      body: JSON.stringify(config),
    }),
  scan: (roots: string[], args: string[] = []) =>
    fetch('/api/scan', {
      method: 'POST',
      headers: { 'x-auth-token': token(), 'content-type': 'application/json' },
      body: JSON.stringify({ roots, args }),
    }),
  status: () => request<StatusSnapshot>('/api/status'),
  report: () => request<Report>('/api/report'),
  authors: () => request<Authors>('/api/report/authors'),
  blame: () =>
    fetch('/api/report/authors', {
      method: 'POST',
      headers: { 'x-auth-token': token() },
    }),
  languages: () => request<LangInfo[]>('/api/languages'),
  fsList: (path: string) =>
    request<DirListing>(`/api/fs/list?path=${encodeURIComponent(path)}&token=${token()}`),
  fsDrives: () => request<string[]>(`/api/fs/drives?token=${token()}`),
  exportUrl: (format: string) => `/api/export?format=${format}&token=${token()}`,
}
