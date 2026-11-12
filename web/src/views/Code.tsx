import { useMemo, useState } from 'react'
import { useStore } from '../store/store'
import { catalog } from '../lib/i18n'
import { DataTable } from '../components/widgets'
import { shortPath } from '../lib/short'
import type { FileRow, FunctionRow } from '../lib/types'

// The code view: the file table and the function table, each with its own
// filter box, mirroring the GUI's sort-and-filter behaviour.
export function Code() {
  const lang = useStore((state) => state.lang)
  const t = catalog(lang)
  const report = useStore((state) => state.report)
  const [fileFilter, setFileFilter] = useState('')
  const [functionFilter, setFunctionFilter] = useState('')
  const [limit, setLimit] = useState(100)
  const [tab, setTab] = useState<'files' | 'functions'>('files')

  // No row cap here: the table sorts the full in-memory report and applies
  // the "top N" selector after the sort, so every column competes fairly.
  const files = useMemo(() => {
    if (!report) return []
    return report.files.filter(
      (file) =>
        !fileFilter ||
        file.path.toLowerCase().includes(fileFilter.toLowerCase()),
    )
  }, [report, fileFilter])

  const functions = useMemo(() => {
    if (!report) return []
    return report.functions.filter(
      (fn) =>
        !functionFilter ||
        `${fn.path} ${fn.name}`.toLowerCase().includes(functionFilter.toLowerCase()),
    )
  }, [report, functionFilter])

  if (!report) {
    return <div className="view-empty">{t.hint_no_scan}</div>
  }

  const fileColumns = [
    { key: 'path', label: t.col_path, value: (row: FileRow) => row.path },
    { key: 'lang', label: t.col_lang, value: (row: FileRow) => row.lang },
    { key: 'code', label: t.col_code, numeric: true, value: (row: FileRow) => row.code },
    { key: 'comment', label: t.col_comment, numeric: true, value: (row: FileRow) => row.comment },
    { key: 'blank', label: t.col_blank, numeric: true, value: (row: FileRow) => row.blank },
    { key: 'test', label: t.col_test, numeric: true, value: (row: FileRow) => row.test },
    { key: 'complexity', label: t.col_complexity, numeric: true, value: (row: FileRow) => row.complexity },
    {
      key: 'smells',
      label: t.col_smells,
      numeric: true,
      value: (row: FileRow) => row.smells.total,
      render: (row: FileRow) => (
        <span title={`${row.smells.unwrap} unwrap · ${row.smells.expect} expect · ${row.smells.panic} panic · ${row.smells.todo} todo · ${row.smells.unimplemented} unimplemented`}>
          {row.smells.total.toLocaleString()}
        </span>
      ),
    },
  ]

  // Function paths arrive absolute; the shared helper cuts the scan root off
  // so the column names the file.
  const short = (path: string) => shortPath(report.roots, path)

  const functionColumns = [
    { key: 'name', label: t.col_function, value: (row: FunctionRow) => row.name },
    {
      key: 'path',
      label: t.col_path,
      value: (row: FunctionRow) => short(row.path),
      render: (row: FunctionRow) => short(row.path),
    },
    { key: 'code', label: t.col_code, numeric: true, value: (row: FunctionRow) => row.code },
    { key: 'complexity', label: t.col_complexity, numeric: true, value: (row: FunctionRow) => row.complexity },
    { key: 'cognitive', label: t.col_cognitive, numeric: true, value: (row: FunctionRow) => row.cognitive },
    { key: 'params', label: t.col_params, numeric: true, value: (row: FunctionRow) => row.params },
    { key: 'depth', label: t.col_depth, numeric: true, value: (row: FunctionRow) => row.depth },
  ]

  return (
    <div className="view">
      <div className="toolbar">
        <div className="tabs">
          <button className={`tab ${tab === 'files' ? 'active' : ''}`} onClick={() => setTab('files')}>
            {t.tab_files} · {report.files.length.toLocaleString()}
          </button>
          <button
            className={`tab ${tab === 'functions' ? 'active' : ''}`}
            onClick={() => setTab('functions')}
          >
            {t.tab_functions} · {report.function_count.toLocaleString()}
          </button>
        </div>
        <input
          className="text-input"
          placeholder={tab === 'files' ? t.hint_filter_files : t.hint_filter_functions}
          value={tab === 'files' ? fileFilter : functionFilter}
          onChange={(event) =>
            tab === 'files' ? setFileFilter(event.target.value) : setFunctionFilter(event.target.value)
          }
        />
        <select
          className="select"
          value={limit}
          onChange={(event) => setLimit(Number(event.target.value))}
        >
          <option value={100}>{t.limit_100}</option>
          <option value={300}>{t.limit_300}</option>
          <option value={1000}>{t.limit_1000}</option>
          <option value={0}>{t.limit_all}</option>
        </select>
      </div>
      {tab === 'files' ? (
        <DataTable
          rows={files}
          columns={fileColumns}
          rowKey={(row) => row.path}
          initialSort="code"
          limit={limit || undefined}
        />
      ) : (
        <DataTable
          rows={functions}
          columns={functionColumns}
          rowKey={(row) => `${row.path}:${row.line}:${row.name}`}
          initialSort="code"
          limit={limit || undefined}
        />
      )}
    </div>
  )
}
