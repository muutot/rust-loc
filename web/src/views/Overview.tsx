import { useState } from 'react'
import { useStore } from '../store/store'
import { catalog, fill } from '../lib/i18n'
import { BarChart, CompositionBar, KpiCard, LineBar } from '../components/widgets'
import { FolderPicker } from '../components/FolderPicker'

// The landing view: the scan roots, the KPI strip, and the headline charts —
// composition, history, languages, authors, modules. Everything that judges
// the code instead of counting it lives on the Quality view.
export function Overview() {
  const lang = useStore((state) => state.lang)
  const t = catalog(lang)
  const report = useStore((state) => state.report)
  const authors = useStore((state) => state.authors)
  const config = useStore((state) => state.config)
  const saveConfig = useStore((state) => state.saveConfig)
  const [pickerOpen, setPickerOpen] = useState(false)

  // The roots live on the landing page now: add or remove them here, the
  // change saves right away and the next scan picks it up.
  const setRoots = (roots: string[]) => {
    if (config) {
      void saveConfig({ ...config, roots }).catch(() => undefined)
    }
  }
  const rootsPanel = config && (
    <div className="panel">
      <div className="panel-head">
        <div className="panel-title">{t.section_roots}</div>
        <button className="button small" onClick={() => setPickerOpen(true)}>
          {t.button_add_folder}
        </button>
      </div>
      {config.roots.length > 0 && (
        <div className="roots">
          {config.roots.map((root) => (
            <div className="root-row" key={root}>
              <span className="root-path">{root}</span>
              <button
                className="button small"
                onClick={() => setRoots(config.roots.filter((r) => r !== root))}
              >
                {t.button_remove}
              </button>
            </div>
          ))}
        </div>
      )}
      {report &&
        report.roots.join('|').toLowerCase() !== config.roots.join('|').toLowerCase() && (
          <div className="stale-roots">
            <div className="stale-caption">{t.hint_last_roots}</div>
            <div className="roots">
              {report.roots.map((root) => (
                <div className="root-row" key={root}>
                  <span className="root-path">{root}</span>
                </div>
              ))}
            </div>
          </div>
        )}
      <FolderPicker
        open={pickerOpen}
        onClose={() => setPickerOpen(false)}
        onPick={(path) => {
          if (config && !config.roots.includes(path)) {
            setRoots([...config.roots, path])
          }
        }}
      />
    </div>
  )

  if (!report) {
    return (
      <div className="view">
        {rootsPanel}
        <div className="view-empty">{t.hint_no_scan}</div>
      </div>
    )
  }
  const summary = report.summary
  const modules = new Map<string, number>()
  for (const file of report.files) {
    modules.set(file.module, (modules.get(file.module) ?? 0) + file.code)
  }
  // Modules with no code left (documentation-only folders) read as rows of
  // zeros; the chart ranks code, so they carry no information.
  const topModules = [...modules.entries()]
    .filter(([, value]) => value > 0)
    .sort((a, b) => b[1] - a[1])
    .slice(0, 12)
    .map(([label, value]) => [label, value] as [string, number])

  // The line-composition strip: the same four segments the GUI overview and
  // the SVG card draw, in the same order.
  const composition = [
    { name: t.seg_code, value: summary.code, color: '#4f8cff' },
    { name: t.seg_doc, value: summary.doc, color: '#9b7bff' },
    { name: t.seg_comment, value: summary.comment - summary.doc, color: '#2f9e77' },
    { name: t.seg_blank, value: summary.blank, color: '#5b6373' },
  ]

  // The languages the scan actually read, largest by total lines first — the
  // same rows the `--languages` table prints. Lines, not code: a
  // documentation-heavy tree (Markdown counts as doc lines) would otherwise
  // show rows of zeros and look like nothing was read.
  const languageRows = [...report.languages].sort((a, b) => b.lines - a.lines)
  const languageTotal = languageRows.reduce((sum, row) => sum + row.lines, 0)

  // Code change between the two newest history samples, for the trend caption.
  const history = report.history
  const codeDelta =
    history.length > 1 ? history[history.length - 1].code - history[history.length - 2].code : 0

  return (
    <div className="view">
      <div className="overview-layout">
        <div className="kpi-strip cards-2col">
          <KpiCard
            label={t.card_total_lines}
            value={summary.physical.toLocaleString()}
            hint={t.card_physical}
          />
          <KpiCard label={t.card_code} value={summary.code.toLocaleString()} />
          <KpiCard
            label={t.card_comment}
            value={summary.comment.toLocaleString()}
            hint={fill(t.hint_of_lines, {
              share: `${summary.lines > 0 ? ((summary.comment / summary.lines) * 100).toFixed(1) : '0'}%`,
            })}
          />
          <KpiCard label={t.card_doc} value={summary.doc.toLocaleString()} />
          <KpiCard
            label={t.card_test}
            value={(report.excluded_file_lines + report.excluded_item_lines).toLocaleString()}
            tone="ok"
            hint={t.hint_test_all}
          />
          <KpiCard
            label={t.card_test_fns}
            value={summary.test_fns_all.toLocaleString()}
            hint={t.hint_test_fns_all}
          />
          <KpiCard
            label={t.card_complexity}
            value={summary.complexity.toLocaleString()}
            hint={t.hint_complexity}
          />
          <KpiCard label={t.card_cognitive} value={summary.cognitive.toLocaleString()} />
          <KpiCard label={t.card_blank} value={summary.blank.toLocaleString()} />
          <KpiCard
            label={t.card_smells}
            value={summary.smells.total.toLocaleString()}
            tone={summary.smells.total > 0 ? 'warn' : 'ok'}
          />
          <KpiCard label={t.card_files} value={report.files.length.toLocaleString()} />
          <KpiCard label={t.card_functions} value={report.function_count.toLocaleString()} />
          {summary.unsafe_kw > 0 && (
            <KpiCard
              label={t.card_unsafe}
              value={summary.unsafe_kw.toLocaleString()}
              hint={t.hint_unsafe}
            />
          )}
          {summary.uloc !== null && (
            <KpiCard
              label={t.card_uloc}
              value={summary.uloc.toLocaleString()}
              hint={fill(t.hint_uloc, {
                share: `${summary.lines > 0 ? ((summary.uloc / summary.lines) * 100).toFixed(1) : '0'}%`,
              })}
            />
          )}
          {report.duplicates && (
            <KpiCard
              label={t.dupes_percent}
              value={`${report.duplicates.percent.toFixed(1)}%`}
            />
          )}
          {report.cocomo && (
            <>
              <KpiCard
                label={t.cocomo_effort}
                value={report.cocomo.effort.toFixed(1)}
                hint={t.unit_person_months}
                tone="info"
              />
              <KpiCard
                label={t.cocomo_schedule}
                value={report.cocomo.schedule.toFixed(1)}
                hint={t.unit_months}
              />
              <KpiCard label={t.cocomo_people} value={report.cocomo.people.toFixed(1)} />
              <KpiCard
                label={t.cocomo_cost}
                value={`$${Math.round(report.cocomo.cost).toLocaleString()}`}
              />
            </>
          )}
        </div>
        <div className="view-main">
          {rootsPanel}

          <div className="panel">
            <div className="panel-title">{t.chart_composition}</div>
            <CompositionBar parts={composition} />
          </div>

          {report.history.length > 1 && (
            <div className="panel">
              <div className="panel-head">
                <div className="panel-title">{t.chart_history}</div>
                <span
                  className={`muted ${
                    codeDelta >= 0 ? 'delta-up' : 'delta-down'
                  }`}
                >
                  {t.card_delta} {codeDelta >= 0 ? '+' : ''}
                  {codeDelta.toLocaleString()}
                </span>
              </div>
              <LineBar samples={report.history} />
            </div>
          )}

          <div className="panel-row">
            <div className="panel">
              <div className="panel-title">{t.chart_languages}</div>
              <BarChart
                rows={languageRows.slice(0, 15).map((row) => [row.name, row.lines])}
                total={languageTotal}
              />
            </div>
            <div className="panel">
              <div className="panel-title">{t.chart_authors}</div>
              {authors === null ? (
                <div className="view-empty">{t.hint_no_authors}</div>
              ) : authors.available ? (
                <BarChart rows={authors.rows.slice(0, 15).map((row) => [row.name, row.code])} />
              ) : (
                <div className="view-empty">{t.hint_no_git}</div>
              )}
            </div>
          </div>

      <div className="panel">
        <div className="panel-title">{t.chart_modules}</div>
        <BarChart rows={topModules} />
      </div>

      {report.diff && (
        <div className="panel">
          <div className="panel-title">
            {t.panel_diff} · {report.diff.path}
          </div>
          <div className="metric-grid">
            <span>{t.diff_code}</span>
            <b className={report.diff.new.code >= report.diff.old.code ? 'delta-up' : 'delta-down'}>
              {report.diff.old.code.toLocaleString()} → {report.diff.new.code.toLocaleString()} (
              {report.diff.new.code - report.diff.old.code >= 0 ? '+' : ''}
              {(report.diff.new.code - report.diff.old.code).toLocaleString()})
            </b>
          </div>
        </div>
      )}
        </div>
      </div>
    </div>
  )
}
