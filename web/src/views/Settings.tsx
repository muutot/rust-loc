import { useEffect, useRef, useState } from 'react'
import { api } from '../lib/api'
import { useStore } from '../store/store'
import { catalog } from '../lib/i18n'
import type { Config, LangInfo } from '../lib/types'

// The settings view: every change lands in the saved config right away, so
// the Scan button in the header always reads the settings on screen.
export function Settings() {
  const lang = useStore((state) => state.lang)
  const t = catalog(lang)
  const config = useStore((state) => state.config)
  const saveConfig = useStore((state) => state.saveConfig)
  const [draft, setDraft] = useState<Config | null>(config)
  const [langs, setLangs] = useState<LangInfo[]>([])
  const timer = useRef<number | undefined>(undefined)
  const pending = useRef(false)

  // Server state only syncs into the draft when nothing is being saved,
  // otherwise a slow PUT response would clobber in-flight typing.
  useEffect(() => {
    if (!pending.current) {
      setDraft(config)
    }
  }, [config])
  useEffect(() => {
    api.languages().then(setLangs).catch(() => undefined)
  }, [])

  // Debounced persist: typing in a field must not fire a PUT per keystroke.
  const persist = (next: Config) => {
    window.clearTimeout(timer.current)
    timer.current = window.setTimeout(() => {
      pending.current = true
      void saveConfig(next)
        .catch(() => undefined)
        .then(() => {
          pending.current = false
        })
    }, 400)
  }

  // Switching views unmounts the panel while a debounced save may still be
  // pending. Flush it with the newest draft now: letting the timer fire
  // later would overwrite server state written meanwhile (roots edited on
  // the overview) with this stale snapshot.
  const latest = useRef(draft)
  latest.current = draft
  useEffect(
    () => () => {
      if (timer.current) {
        window.clearTimeout(timer.current)
        timer.current = undefined
        if (latest.current) {
          void useStore.getState().saveConfig(latest.current).catch(() => undefined)
        }
      }
    },
    [],
  )

  // The header's language switch also lands in the draft and is saved.
  useEffect(() => {
    setDraft((current) => {
      if (!current || current.lang === lang) {
        return current
      }
      const next = { ...current, lang }
      persist(next)
      return next
    })
  }, [lang])

  if (!draft) {
    return <div className="view-empty">…</div>
  }
  const patch = (part: Partial<Config>) => {
    const next = { ...draft, ...part }
    setDraft(next)
    persist(next)
  }
  const toggleSource = (slug: string) => {
    const has = draft.sources.includes(slug)
    patch({ sources: has ? draft.sources.filter((s) => s !== slug) : [...draft.sources, slug] })
  }

  const checkboxes: [keyof Config, string][] = [
    ['gitignore', t.opt_gitignore],
    ['default_ignores', t.opt_default_ignores],
    ['include_tests', t.opt_include_tests],
    ['complexity', t.opt_complexity],
    ['uloc', t.opt_uloc],
    ['exclude_generated', t.opt_exclude_generated],
    ['cocomo', t.opt_cocomo],
    ['authors', t.opt_authors],
  ]

  return (
    <div className="view">
      <div className="panel">
        <div className="panel-title">{t.section_scope}</div>
        <div className="checks">
          {checkboxes.map(([key, label]) => (
            <label className="check" key={key}>
              <input
                type="checkbox"
                checked={Boolean(draft[key])}
                onChange={(event) => patch({ [key]: event.target.checked } as Partial<Config>)}
              />
              {label}
            </label>
          ))}
        </div>
      </div>

      <div className="panel">
        <div className="panel-title">{t.section_filters}</div>
        <div className="field-grid">
          <label>{t.field_exclude_ext}</label>
          <input
            className="text-input"
            value={draft.exclude_ext.join(' ')}
            placeholder={t.hint_separator}
            onChange={(event) =>
              patch({ exclude_ext: event.target.value.split(/[\s,，]+/).filter(Boolean) })
            }
          />
          <label>{t.field_max_size}</label>
          <input
            className="text-input"
            type="number"
            value={draft.max_file_size}
            onChange={(event) => patch({ max_file_size: Number(event.target.value) || 0 })}
          />
          <label>{t.field_exclude}</label>
          <input
            className="text-input"
            value={draft.exclude.join(' ')}
            placeholder={t.hint_separator}
            onChange={(event) =>
              patch({ exclude: event.target.value.split(/[\s,，]+/).filter(Boolean) })
            }
          />
          <label>{t.field_fail_if}</label>
          <input
            className="text-input"
            value={draft.fail_if.join(', ')}
            placeholder={t.hint_fail_if}
            onChange={(event) =>
              // A spec may carry spaces ("complexity >= 5000"), so only
              // commas and semicolons separate entries.
              patch({ fail_if: event.target.value.split(/[,，;]+/).map((s) => s.trim()).filter(Boolean) })
            }
          />
          <label>{t.field_extra_dirs}</label>
          <textarea
            className="text-input"
            rows={3}
            value={draft.extra_dirs.join('\n')}
            placeholder={t.hint_extra_dirs}
            onChange={(event) =>
              patch({ extra_dirs: event.target.value.split(/[\n,，]+/).filter(Boolean) })
            }
          />
        </div>
      </div>

      <div className="panel">
        <div className="panel-title">{t.section_languages}</div>
        <div className="checks checks-grid">
          {langs.map((info) => (
            <label className="check" key={info.slug}>
              <input
                type="checkbox"
                checked={draft.sources.includes(info.slug)}
                onChange={() => toggleSource(info.slug)}
              />
              {info.name}
            </label>
          ))}
        </div>
      </div>
    </div>
  )
}
