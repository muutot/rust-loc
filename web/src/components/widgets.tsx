import { useState } from 'react'
import type { ReactNode } from 'react'

import { useStore } from '../store/store'
import { catalog } from '../lib/i18n'

// One KPI tile: the big number, its label with the small hint beside it. An
// optional accent hue lets related cards read as one family (tests green,
// smells amber, effort teal) without a legend.
export function KpiCard(props: {
  label: string
  value: string
  hint?: string
  tone?: 'default' | 'ok' | 'warn' | 'info'
  delta?: number
}) {
  const toneClass = props.tone && props.tone !== 'default' ? ` tone-${props.tone}` : ''
  return (
    <div className={`kpi${toneClass}`}>
      <div className="kpi-head">
        <span className="kpi-label">{props.label}</span>
        {props.hint && <span className="kpi-hint">({props.hint})</span>}
        {typeof props.delta === 'number' && props.delta !== 0 && (
          <span className={`kpi-delta ${props.delta > 0 ? 'delta-up' : 'delta-down'}`}>
            {props.delta > 0 ? '+' : ''}
            {props.delta.toLocaleString()}
          </span>
        )}
      </div>
      <div className="kpi-value">{props.value}</div>
    </div>
  )
}

// Horizontal bars, the svg.rs style: label left, bar middle, number right.
// Rows are `(label, value)`; the widest bar fills the track. `total` turns
// the value into a share, the way the language table prints one.
export function BarChart(props: {
  rows: [string, number][]
  unit?: string
  total?: number
}) {
  const max = Math.max(1, ...props.rows.map(([, value]) => value))
  const total = props.total ?? max
  return (
    <div className="bars">
      {props.rows.map(([label, value]) => {
        const share = total > 0 ? value / total : 0
        return (
          <div className="bar-row" key={label}>
            <span className="bar-label" title={label}>
              {label}
            </span>
            <div className="bar-rest">
              <svg className="bar-track" height="14">
                <rect
                  className="bar-fill"
                  x="0"
                  y="2"
                  height="10"
                  rx="3"
                  width={`${Math.max(0.5, (value / max) * 100)}%`}
                />
              </svg>
              <span className="bar-value">
                {value.toLocaleString()}
                {props.unit ?? ''}
                {props.total !== undefined && (
                  <span className="bar-share"> · {(share * 100).toFixed(1)}%</span>
                )}
              </span>
            </div>
          </div>
        )
      })}
    </div>
  )
}

// The stacked composition bar: one strip of code / doc / comment / blank, the
// way the GUI's overview and the SVG card draw it. Segments keep a hairline
// gap so the boundaries survive on dark backgrounds. The legend already
// names every segment with its count and share, so hovering only highlights
// — nothing new appears and the layout never shifts.
export function CompositionBar(props: {
  parts: { name: string; value: number; color: string }[]
}) {
  const total = props.parts.reduce((sum, part) => sum + part.value, 0)
  const [hover, setHover] = useState<number | null>(null)
  return (
    <div>
      <div className="comp-bar" role="img">
        {props.parts.map((part, index) => {
          const width = total > 0 ? (part.value / total) * 100 : 0
          return (
            <div
              key={part.name}
              className="comp-seg"
              style={{ width: `${width}%`, background: part.color }}
              onMouseEnter={() => setHover(index)}
              onMouseLeave={() => setHover(null)}
            />
          )
        })}
      </div>
      <div className="comp-legend">
        {props.parts.map((part, index) => (
          <span
            key={part.name}
            className={`comp-item ${hover === index || hover === null ? '' : 'dim'}`}
            onMouseEnter={() => setHover(index)}
            onMouseLeave={() => setHover(null)}
          >
            <i className="comp-dot" style={{ background: part.color }} />
            {part.name}
            <b>{part.value.toLocaleString()}</b>
            <span className="comp-share">
              {total > 0 ? `${((part.value / total) * 100).toFixed(1)}%` : '—'}
            </span>
          </span>
        ))}
      </div>
    </div>
  )
}

// The history sparkline: one column per scan, code lines tall.
export function LineBar(props: { samples: { code: number; when: number }[] }) {
  const samples = props.samples.slice(-30)
  if (samples.length < 2) return null
  const width = 480
  const height = 90
  const max = Math.max(...samples.map((sample) => sample.code), 1)
  const min = Math.min(...samples.map((sample) => sample.code))
  const span = Math.max(1, max - min)
  const barWidth = width / samples.length
  return (
    <svg viewBox={`0 0 ${width} ${height}`} className="spark">
      {samples.map((sample, index) => {
        // Baseline at the smallest sample, not zero: ±1 line of churn on a
        // 200k-line tree must still move the column.
        const h = 8 + ((sample.code - min) / span) * (height - 16)
        return (
          <rect
            key={sample.when}
            x={index * barWidth + 1}
            y={height - h}
            width={Math.max(2, barWidth - 2)}
            height={h}
            rx="2"
            className={`spark-bar ${index === samples.length - 1 ? 'spark-latest' : ''}`}
          >
            <title>
              {new Date(sample.when * 1000).toLocaleString()} · {sample.code.toLocaleString()}
            </title>
          </rect>
        )
      })}
    </svg>
  )
}

// A sortable table: click the header to sort (first click descending, again
// flips), `render` draws a cell. `initialSort`/`initialDesc` set the start.
export interface Column<Row> {
  key: string
  label: string
  numeric?: boolean
  value: (row: Row) => number | string
  render?: (row: Row) => ReactNode
}

export function DataTable<Row>(props: {
  rows: Row[]
  columns: Column<Row>[]
  rowKey: (row: Row) => string
  initialSort?: string
  initialDesc?: boolean
  filter?: (row: Row) => boolean
  /** Row cap applied after sorting, so the "top N" shows the N the sort picked. */
  limit?: number
}) {
  const [sortKey, setSortKey] = useState(props.initialSort ?? '')
  const [desc, setDesc] = useState(props.initialDesc ?? true)

  const click = (key: string) => {
    if (key === sortKey) {
      setDesc(!desc)
    } else {
      setSortKey(key)
      setDesc(true)
    }
  }

  const rows = props.filter ? props.rows.filter(props.filter) : props.rows
  const column = props.columns.find((candidate) => candidate.key === sortKey)
  const sorted = column
    ? [...rows].sort((left, right) => {
        const a = column.value(left)
        const b = column.value(right)
        const order =
          typeof a === 'number' && typeof b === 'number'
            ? a - b
            : String(a).localeCompare(String(b))
        return desc ? -order : order
      })
    : rows
  // The cap lands after the sort: "top 100" means the first 100 rows of
  // whatever column is sorted, not the first 100 rows rearranged.
  const visible = props.limit ? sorted.slice(0, props.limit) : sorted

  return (
    <div className="table-wrap">
      <table className="data-table">
        <thead>
          <tr>
            {props.columns.map((column) => (
              <th
                key={column.key}
                className={column.numeric ? 'num' : ''}
                onClick={() => click(column.key)}
              >
                {column.label}
                {column.key === sortKey && (
                  <span className="sort-mark">{desc ? ' ▼' : ' ▲'}</span>
                )}
              </th>
            ))}
          </tr>
        </thead>
        <tbody>
          {visible.map((row) => (
            <tr key={props.rowKey(row)}>
              {props.columns.map((column) => (
                <td key={column.key} className={column.numeric ? 'num' : ''}>
                  {column.render ? column.render(row) : String(column.value(row))}
                </td>
              ))}
            </tr>
          ))}
        </tbody>
      </table>
      {visible.length === 0 && <div className="empty">—</div>}
    </div>
  )
}

// The scan/blame banner: live progress while either stage runs.
export function ProgressBanner() {
  const status = useStore((state) => state.status)
  const t = useStore((state) => catalog(state.lang))
  const scan = status.scan
  const authors = status.authors
  if (scan.kind === 'failed') {
    return <div className="banner banner-error">{t.status_scan_failed}: {scan.error}</div>
  }
  // A scan or a blame pass veils the page: dimmed backdrop, centered card.
  let text = ''
  let path = ''
  let percent = 0
  if (scan.kind === 'scanning') {
    text = `${scan.stage === 'blame' ? t.status_blaming : t.status_scanning} · ${scan.done}/${scan.total} · ${
      scan.total > 0 ? Math.round((scan.done / scan.total) * 100) : 0
    }%`
    path = scan.path
    percent = scan.total > 0 ? Math.round((scan.done / scan.total) * 100) : 0
  } else if (authors.kind === 'blaming') {
    text = `${t.status_blaming} · ${authors.done}/${authors.total} · ${
      authors.total > 0 ? Math.round((authors.done / authors.total) * 100) : 0
    }%`
    percent = authors.total > 0 ? Math.round((authors.done / authors.total) * 100) : 0
  } else {
    return null
  }
  return (
    <div className="progress-veil">
      <div className="progress-card">
        <div className="banner-text">{text}</div>
        {path && <div className="banner-path">{path}</div>}
        <div className="banner-track">
          <div className="banner-fill" style={{ width: `${percent}%` }} />
        </div>
      </div>
    </div>
  )
}
