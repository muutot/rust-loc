import { useStore } from '../store/store'
import { catalog } from '../lib/i18n'
import { DataTable } from '../components/widgets'
import type { AuthorRow } from '../lib/types'

// The author view: per-author totals first, then the month-by-month activity
// grid, then one table naming who wrote what in each file. The blame runs
// from the server; this view only draws.
export function Authors() {
  const lang = useStore((state) => state.lang)
  const t = catalog(lang)
  const authors = useStore((state) => state.authors)
  const status = useStore((state) => state.status)
  const blame = useStore((state) => state.blame)
  const busy = status.authors.kind === 'blaming'

  const columns = [
    { key: 'name', label: t.col_author, value: (row: AuthorRow) => row.name },
    { key: 'code', label: t.col_code, numeric: true, value: (row: AuthorRow) => row.code },
    { key: 'comment', label: t.col_comment, numeric: true, value: (row: AuthorRow) => row.comment },
    { key: 'test', label: t.col_test, numeric: true, value: (row: AuthorRow) => row.test },
    { key: 'files', label: t.col_files, numeric: true, value: (row: AuthorRow) => row.files },
  ]

  const fileRows = (authors?.files ?? []).map((file) => ({
    path: file.path,
    lead: file.shares[0],
    total: file.shares.reduce((sum, share) => sum + share.code, 0),
  }))

  // The month grid: one row per author, one column per month. Cells shade
  // darker as the commit count grows, so quiet stretches read at a glance.
  const activity = authors?.activity
  const peak = Math.max(1, ...(activity?.rows.flatMap((row) => row.commits) ?? [0]))

  return (
    <div className="view">
      {authors && !authors.available && (
        <div className="view-empty">{t.hint_no_git}</div>
      )}
      {authors?.available && (
        <>
          {authors.untracked > 0 && (
            <div className="panel">
              <div className="panel-title">
                {t.hint_untracked}: {authors.untracked.toLocaleString()}
              </div>
            </div>
          )}
          <div className="panel">
            <div className="panel-title">{t.table_authors}</div>
            <DataTable
              rows={authors.rows}
              columns={columns}
              rowKey={(row) => row.email}
              initialSort="code"
            />
          </div>
          {activity && activity.months.length > 0 && activity.rows.length > 0 && (
            <div className="panel">
              <div className="panel-title">{t.panel_activity}</div>
              <div className="table-wrap">
                <table className="data-table">
                  <thead>
                    <tr>
                      <th>{t.col_author}</th>
                      {activity.months.map((month) => (
                        <th key={month} className="num">
                          {month.slice(2)}
                        </th>
                      ))}
                    </tr>
                  </thead>
                  <tbody>
                    {activity.rows.map((row) => (
                      <tr key={row.name}>
                        <td>{row.name}</td>
                        {row.commits.map((count, index) => {
                          const alpha = count === 0 ? 0 : 0.15 + (count / peak) * 0.75
                          return (
                            <td
                              key={index}
                              className="num"
                              style={{ background: `rgba(96, 165, 250, ${alpha.toFixed(2)})` }}
                            >
                              {count === 0 ? '·' : count.toLocaleString()}
                            </td>
                          )
                        })}
                      </tr>
                    ))}
                  </tbody>
                </table>
              </div>
            </div>
          )}
          <div className="panel">
            <div className="panel-title">{t.table_files}</div>
            <DataTable
              rows={fileRows}
              columns={[
                { key: 'path', label: t.col_path, value: (row: (typeof fileRows)[number]) => row.path },
                { key: 'lead', label: t.col_lead, value: (row: (typeof fileRows)[number]) => row.lead?.name ?? '' },
                {
                  key: 'share',
                  label: t.col_share,
                  value: (row: (typeof fileRows)[number]) =>
                    row.total > 0 ? Math.round((row.lead?.code ?? 0) / row.total * 100) : 0,
                  render: (row: (typeof fileRows)[number]) =>
                    `${Math.round(((row.lead?.code ?? 0) / Math.max(1, row.total)) * 100)}%`,
                },
                { key: 'code', label: t.col_code, numeric: true, value: (row: (typeof fileRows)[number]) => row.total },
              ]}
              rowKey={(row) => row.path}
            />
          </div>
        </>
      )}
      {!authors?.available && !busy && (
        <button className="button primary" onClick={() => void blame()}>
          {t.button_blame}
        </button>
      )}
    </div>
  )
}
