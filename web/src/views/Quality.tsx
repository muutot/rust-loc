import { useStore } from '../store/store'
import { catalog, fill } from '../lib/i18n'
import { DataTable } from '../components/widgets'
import type { CouplingPair, Dupes, FileRow, Gate, Hotspot } from '../lib/types'
import { shortPath } from '../lib/short'

// The hotspot ranking: complexity × churn, worst first, the same columns the
// CLI's `--hotspots` table prints. Labels resolve at render time so the
// language switch re-renders them like every other string.
function hotspotColumns(t: ReturnType<typeof catalog>) {
  return [
    { key: 'path', label: t.col_path, value: (row: Hotspot) => row.path },
    { key: 'code', label: t.col_code, numeric: true, value: (row: Hotspot) => row.code },
    {
      key: 'complexity',
      label: t.col_complexity,
      numeric: true,
      value: (row: Hotspot) => row.complexity,
    },
    { key: 'commits', label: t.col_commits, numeric: true, value: (row: Hotspot) => row.commits },
    { key: 'churn', label: t.col_churn, numeric: true, value: (row: Hotspot) => row.churn },
    { key: 'score', label: t.col_score, numeric: true, value: (row: Hotspot) => row.score },
  ]
}

// The smell breakdown, one row per file: the total plus every counter the
// CLI's smells block tracks, so a reviewer can see what kind of escape a
// file carries without opening it.
function smellColumns(t: ReturnType<typeof catalog>) {
  return [
    { key: 'path', label: t.col_path, value: (row: FileRow) => row.path },
    {
      key: 'total',
      label: t.col_smells,
      numeric: true,
      value: (row: FileRow) => row.smells.total,
      render: (row: FileRow) => (
        <b className={row.smells.total > 0 ? 'delta-down' : undefined}>
          {row.smells.total.toLocaleString()}
        </b>
      ),
    },
    { key: 'unwrap', label: t.col_unwrap, numeric: true, value: (row: FileRow) => row.smells.unwrap },
    { key: 'expect', label: t.col_expect, numeric: true, value: (row: FileRow) => row.smells.expect },
    { key: 'panic', label: t.col_panic, numeric: true, value: (row: FileRow) => row.smells.panic },
    { key: 'todo', label: t.col_todo, numeric: true, value: (row: FileRow) => row.smells.todo },
    {
      key: 'unimplemented',
      label: t.col_unimplemented,
      numeric: true,
      value: (row: FileRow) => row.smells.unimplemented,
    },
    { key: 'allow', label: t.col_allow, numeric: true, value: (row: FileRow) => row.smells.allow },
  ]
}

// One row per copied block: its length and every site that holds it. Sites
// render as `path:line`, the way `--duplicates` lists them.
function dupeColumns(t: ReturnType<typeof catalog>, short: (path: string) => string) {
  return [
    {
      key: 'lines',
      label: t.col_lines,
      numeric: true,
      value: (row: Dupes['clusters'][number]) => row.lines,
    },
    {
      key: 'sites',
      label: t.col_sites,
      value: (row: Dupes['clusters'][number]) =>
        row.sites.map((s) => `${short(s.path)}:${s.line}`).join('  ·  '),
    },
  ]
}

// The gates as one row per rule: spec, measured value against the limit, and
// the worst offender when the rule names files.
function gateColumns(t: ReturnType<typeof catalog>) {
  return [
    { key: 'spec', label: t.field_fail_if, value: (row: Gate) => row.spec },
    {
      key: 'value',
      label: 'Value',
      numeric: true,
      value: (row: Gate) => row.value,
      render: (row: Gate) => (
        <span className={row.ok ? 'delta-up' : 'delta-down'}>
          {row.error ??
            row.value.toLocaleString(undefined, { maximumFractionDigits: 2 })}
        </span>
      ),
    },
    { key: 'limit', label: 'Limit', numeric: true, value: (row: Gate) => row.limit },
    {
      key: 'verdict',
      label: 'OK',
      value: (row: Gate) => (row.ok ? 'pass' : 'fail'),
      render: (row: Gate) =>
        row.error ? (
          <span className="muted">{row.error}</span>
        ) : row.ok ? (
          <span className="delta-up">✓</span>
        ) : (
          <span className="delta-down">✗</span>
        ),
    },
  ]
}

// The quality view: everything that judges the code rather than counts it —
// gates, hotspots, smells, duplicates, maintainability, coupling and the
// files a scan left out. The overview keeps the headline charts; this view
// holds the tables a reviewer works through.
export function Quality() {
  const lang = useStore((state) => state.lang)
  const t = catalog(lang)
  const report = useStore((state) => state.report)

  if (!report) {
    return <div className="view-empty">{t.quality_empty}</div>
  }
  const short = (path: string) => shortPath(report.roots, path)
  const summary = report.summary
  const smellsFiles = [...report.files]
    .filter((file) => file.smells.total > 0)
    .sort((a, b) => b.smells.total - a.smells.total)
  const dupes = report.duplicates

  return (
    <div className="view">
      {report.gates.length > 0 && (
        <div className="panel">
          <div className="panel-title">{t.panel_gates}</div>
          <DataTable
            rows={report.gates}
            columns={gateColumns(t)}
            rowKey={(row: Gate) => row.spec}
            initialSort="spec"
            initialDesc={false}
          />
        </div>
      )}

      {report.hotspots.length > 0 && (
        <div className="panel">
          <div className="panel-title">{t.card_hotspots}</div>
          <DataTable
            rows={report.hotspots}
            columns={hotspotColumns(t)}
            rowKey={(row: Hotspot) => row.path}
            initialSort="score"
          />
        </div>
      )}

      <div className="panel">
        <div className="panel-title">
          {t.panel_smells_detail} ·{' '}
          {fill(t.smells_hint, {
            total: summary.smells.total.toLocaleString(),
            files: smellsFiles.length.toLocaleString(),
            unwrap: summary.smells.unwrap.toLocaleString(),
            expect: summary.smells.expect.toLocaleString(),
            panic: summary.smells.panic.toLocaleString(),
            todo: summary.smells.todo.toLocaleString(),
            allow: summary.smells.allow.toLocaleString(),
          })}
        </div>
        {smellsFiles.length > 0 ? (
          <DataTable
            rows={smellsFiles}
            columns={smellColumns(t)}
            rowKey={(row: FileRow) => row.path}
            initialSort="total"
          />
        ) : (
          <div className="empty">{t.hint_no_smells}</div>
        )}
      </div>

      {dupes && (
        <div className="panel">
          <div className="panel-title">
            {t.panel_dupes} · {dupes.percent.toFixed(1)}% ·{' '}
            {fill(t.dupes_window_hint, { window: dupes.window })}
          </div>
          <div className="metric-grid">
            <span>{t.dupes_percent}</span>
            <b>{dupes.percent.toFixed(1)}%</b>
            <span>{t.dupes_lines}</span>
            <b>{dupes.duplicated_lines.toLocaleString()}</b>
          </div>
          {dupes.clusters.length > 0 && (
            <DataTable
              rows={dupes.clusters}
              columns={dupeColumns(t, short)}
              rowKey={(row: Dupes['clusters'][number]) =>
                `${row.lines}:${row.sites[0]?.path ?? ''}:${row.sites[0]?.line ?? 0}`
              }
              initialSort="lines"
            />
          )}
        </div>
      )}

      <div className="panel-row">
        {report.metrics && (
          <div className="panel">
            <div className="panel-title">{t.panel_metrics}</div>
            <div className="metric-grid">
              <span>{t.metric_index}</span>
              <b>
                {report.metrics.index.toFixed(1)} · {report.metrics.band}
              </b>
              <span>{t.metric_volume}</span>
              <b>{Math.round(report.metrics.volume).toLocaleString()}</b>
              <span>{t.metric_cognitive}</span>
              <b>{report.metrics.cognitive.toLocaleString()}</b>
              <span>{t.metric_tokens}</span>
              <b>{report.metrics.tokens.toLocaleString()}</b>
              <span>{t.metric_methods ?? 'Methods'}</span>
              <b>
                {report.metrics.methods.toLocaleString()} /{' '}
                {report.metrics.arguments.toLocaleString()}
              </b>
            </div>
          </div>
        )}

        {report.coupling && report.coupling.pairs.length > 0 && (
          <div className="panel">
            <div className="panel-title">
              {t.panel_coupling} ·{' '}
              {fill(t.coupling_examined, { n: report.coupling.commits.toLocaleString() })}
            </div>
            <DataTable
              rows={report.coupling.pairs}
              columns={[
                {
                  key: 'pair',
                  label: t.col_pair,
                  value: (row: CouplingPair) => `${row.a} <-> ${row.b}`,
                  render: (row: CouplingPair) => (
                    <>
                      <code>{short(row.a)}</code> ↔ <code>{short(row.b)}</code>
                    </>
                  ),
                },
                {
                  key: 'commits',
                  label: t.col_commits,
                  numeric: true,
                  value: (row: CouplingPair) => row.commits,
                },
              ]}
              rowKey={(row: CouplingPair) => `${row.a}|${row.b}`}
              initialSort="commits"
            />
          </div>
        )}
      </div>

      {report.excluded_paths.length > 0 && (
        <div className="panel">
          <div className="panel-title">
            {t.panel_excluded} · {report.excluded_files.toLocaleString()} {t.unit_files} ·{' '}
            {report.excluded_file_lines.toLocaleString()} {t.unit_lines}
          </div>
          <DataTable
            rows={[...report.excluded_paths]
              .sort((a, b) => b.lines - a.lines)
              .map((row) => ({ ...row, shown: short(row.path) }))}
            columns={[
              {
                key: 'shown',
                label: t.col_path,
                value: (row: (typeof report.excluded_paths)[number] & { shown: string }) =>
                  row.shown,
              },
              {
                key: 'lines',
                label: t.col_lines,
                numeric: true,
                value: (row: (typeof report.excluded_paths)[number] & { shown: string }) =>
                  row.lines,
              },
            ]}
            rowKey={(row) => row.path}
            initialSort="lines"
          />
        </div>
      )}
    </div>
  )
}
