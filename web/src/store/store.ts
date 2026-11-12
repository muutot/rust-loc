// One store drives the page: the saved settings, the live scan state and the
// last report. The SSE stream writes scan/authors; the views read everything.

import { create } from 'zustand'
import { api } from '../lib/api'
import { openEvents } from '../lib/sse'
import type { Authors, Config, Report, StatusSnapshot } from '../lib/types'

interface Store {
  config: Config | null
  report: Report | null
  authors: Authors | null
  status: StatusSnapshot
  error: string
  view: 'overview' | 'quality' | 'authors' | 'code' | 'settings'
  lang: number
  setView: (view: Store['view']) => void
  setLang: (lang: number) => void
  loadConfig: () => Promise<void>
  saveConfig: (config: Config) => Promise<void>
  scan: (roots?: string[], args?: string[]) => Promise<void>
  loadReport: () => Promise<void>
  blame: () => Promise<void>
  connect: () => void
  setError: (error: string) => void
}

const initialStatus: StatusSnapshot = {
  scan: { kind: 'idle' },
  authors: { kind: 'idle' },
}

export const useStore = create<Store>((set, get) => ({
  config: null,
  report: null,
  authors: null,
  status: initialStatus,
  error: '',
  view: 'overview',
  lang: 0,

  setView: (view) => set({ view }),

  setLang: (lang) => set({ lang }),

  loadConfig: async () => {
    const config = await api.config()
    set({ config, lang: config.lang === 1 ? 1 : 0 })
    // A report from an earlier scan is still on the server; show it.
    get().loadReport().catch(() => undefined)
  },

  saveConfig: async (config) => {
    const saved = await api.saveConfig(config)
    set({ config: saved, lang: saved.lang === 1 ? 1 : 0 })
  },

  scan: async (roots, args = []) => {
    const current = get().config
    const target = roots ?? current?.roots ?? []
    set({ error: '' })
    const response = await api.scan(target, args)
    if (!response.ok) {
      set({ error: await response.text() })
    }
  },

  loadReport: async () => {
    // A new report invalidates the author tables of the previous scan: drop
    // them first so a project switch never shows the old project's authors.
    set({ authors: null })
    const report = await api.report()
    set({ report })
    // The author tables load with the page only when the scan enabled them.
    if (report.summary) {
      api
        .authors()
        .then((authors) => set({ authors }))
        .catch(() => undefined)
    }
  },

  blame: async () => {
    const response = await api.blame()
    if (!response.ok) {
      set({ error: await response.text() })
    }
  },

  connect: () =>
    openEvents((snapshot: StatusSnapshot) => {
      const before = get().status.scan.kind
      set({ status: snapshot })
      // A scan that just finished pulls the new report in; the views read it.
      if (snapshot.scan.kind === 'done' && before !== 'done') {
        get()
          .loadReport()
          .catch(() => undefined)
      }
      if (snapshot.authors.kind === 'ready' && get().status.scan.kind === 'done') {
        api
          .authors()
          .then((authors) => set({ authors }))
          .catch(() => undefined)
      }
    }),

  setError: (error) => set({ error }),
}))
