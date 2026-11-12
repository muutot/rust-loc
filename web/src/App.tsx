import { useEffect, useState } from 'react'
import { useStore } from './store/store'
import { catalog, fill, LANG_NAMES } from './lib/i18n'
import { api } from './lib/api'
import { ProgressBanner } from './components/widgets'
import { Overview } from './views/Overview'
import { Quality } from './views/Quality'
import { Authors } from './views/Authors'
import { Code } from './views/Code'
import { Settings } from './views/Settings'

// The formats /api/export serves, in the order the dialog offers them.
const EXPORT_FORMATS: [string, keyof ReturnType<typeof catalog>][] = [
  ['markdown', 'export_markdown'],
  ['json', 'export_json'],
  ['csv', 'export_csv'],
  ['text', 'export_text'],
  ['svg', 'export_svg'],
  ['html', 'export_html'],
]

// F5 rescan: the same key the native shell answers, guarded so it never
// hijacks a keystroke inside an input field.
function useScanShortcut(scan: () => void, enabled: boolean) {
  useEffect(() => {
    if (!enabled) return
    const onKey = (event: KeyboardEvent) => {
      if (event.key === 'F5') {
        event.preventDefault()
        scan()
      }
    }
    window.addEventListener('keydown', onKey)
    return () => window.removeEventListener('keydown', onKey)
  }, [scan, enabled])
}

// The shell: title bar, the four tabs the GUI groups map to, the live banner
// and the active view. The store wires everything, no prop drilling.
export function App() {
  const lang = useStore((state) => state.lang)
  const t = catalog(lang)
  const view = useStore((state) => state.view)
  const setView = useStore((state) => state.setView)
  const connect = useStore((state) => state.connect)
  const loadConfig = useStore((state) => state.loadConfig)
  const scan = useStore((state) => state.scan)
  const error = useStore((state) => state.error)
  const status = useStore((state) => state.status)
  const report = useStore((state) => state.report)
  const config = useStore((state) => state.config)
  const authors = useStore((state) => state.authors)
  const blame = useStore((state) => state.blame)
  const setLang = useStore((state) => state.setLang)
  const setError = useStore((state) => state.setError)
  const [exportOpen, setExportOpen] = useState(false)
  const [scanMode, setScanMode] = useState<'all' | 'files'>('files')
  const hasAuthors = Boolean(authors?.available)
  const scanning = status.scan.kind === 'scanning'
  // F5 rescans with the mode the selector holds, exactly like the button.
  useScanShortcut(() => {
    if (config && !scanning) void scan(undefined, scanMode === 'all' ? ['--authors'] : [])
  }, Boolean(config) && !scanning)
  // The Authors tab follows the data: it appears once a scan or a blame pass
  // has actually produced author data, and disappears when there is none.
  const showAuthorsTab = hasAuthors

  // Fetch the export and hand the browser a blob: a plain link cannot report
  // a failure, this one surfaces it in the error banner.
  const download = async (format: string) => {
    setExportOpen(false)
    try {
      const response = await fetch(api.exportUrl(format))
      if (!response.ok) {
        setError((await response.text()) || `HTTP ${response.status}`)
        return
      }
      const url = URL.createObjectURL(await response.blob())
      const anchor = document.createElement('a')
      anchor.href = url
      anchor.download = `rust-loc.${format === 'markdown' ? 'md' : format}`
      anchor.click()
      URL.revokeObjectURL(url)
    } catch (err) {
      setError(String(err))
    }
  }

  useEffect(() => {
    void loadConfig().catch(() => undefined)
    return connect()
  }, [])

  const tabs: [typeof view, string][] = [
    ['overview', t.tab_overview],
    ['quality', t.tab_quality],
    ...(showAuthorsTab ? [['authors', t.tab_authors] as [typeof view, string]] : []),
    ['code', t.tab_code],
    ['settings', t.tab_settings],
  ]

  // The line under the title: what the last scan read and how long it took,
  // so a report never sits on screen without saying where it came from.
  const subtitle = report
    ? fill(t.status_scanned, {
        files: report.summary.physical >= 0 ? report.files.length : 0,
        seconds: report.elapsed.toFixed(2),
        test: report.summary.test.toLocaleString(),
      })
    : t.no_scan_yet

  return (
    <div className="shell">
      <header className="header">
        <div className="header-brand">
          <div className="header-title">rust-loc</div>
          <div className="header-subtitle">{subtitle}</div>
        </div>
        <div className="header-actions">
          <select
            className="select"
            value={lang}
            onChange={(event) => setLang(Number(event.target.value))}
          >
            {LANG_NAMES.map((name, index) => (
              <option key={name} value={index}>
                {name}
              </option>
            ))}
          </select>
          <select
            className="select"
            value={scanMode}
            onChange={(event) => setScanMode(event.target.value as 'all' | 'files')}
          >
            <option value="all">{t.scan_all}</option>
            <option value="files">{t.scan_files}</option>
          </select>
          {report && report.git_available && !hasAuthors && (
            <button
              className="button download"
              disabled={status.authors.kind === 'blaming'}
              onClick={() => void blame()}
            >
              {t.button_blame}
            </button>
          )}
          <button
            className="button download"
            disabled={!report}
            onClick={() => setExportOpen(true)}
          >
            {t.button_export}
          </button>
          <button
            className="button primary"
            disabled={!config || scanning}
            onClick={() => void scan(undefined, scanMode === 'all' ? ['--authors'] : [])}
          >
            {t.button_scan}
          </button>
        </div>
      </header>
      <ProgressBanner />
      {error && <div className="banner banner-error">{error}</div>}
      <nav className="tabs main-tabs">
        {tabs.map(([key, label]) => (
          <button
            key={key}
            className={`tab ${view === key ? 'active' : ''}`}
            onClick={() => setView(key)}
          >
            {label}
          </button>
        ))}
      </nav>
      <main className="content">
        {view === 'overview' && <Overview />}
        {view === 'quality' && <Quality />}
        {view === 'authors' && (showAuthorsTab ? <Authors /> : <Overview />)}
        {view === 'code' && <Code />}
        {view === 'settings' && <Settings />}
      </main>
      {exportOpen && (
        <div className="modal-veil" onClick={() => setExportOpen(false)}>
          <div className="modal" onClick={(event) => event.stopPropagation()}>
            <div className="modal-title">{t.export_pick_format}</div>
            <div className="dialog-formats">
              {EXPORT_FORMATS.map(([format, labelKey]) => (
                <button
                  key={format}
                  className="chip"
                  onClick={() => void download(format)}
                >
                  {t[labelKey]}
                </button>
              ))}
            </div>
            <div className="modal-actions">
              <button className="button" onClick={() => setExportOpen(false)}>
                {t.picker_cancel}
              </button>
            </div>
          </div>
        </div>
      )}
    </div>
  )
}
