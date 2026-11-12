import { useEffect, useState } from 'react'
import { api } from '../lib/api'
import type { DirListing } from '../lib/types'
import { useStore } from '../store/store'
import { catalog } from '../lib/i18n'

// The folder picker: a drive list, a path box and the directories below the
// current one. It walks the tree through /api/fs/* and never reads a file.
export function FolderPicker(props: { open: boolean; onClose: () => void; onPick: (path: string) => void }) {
  const lang = useStore((state) => state.lang)
  const t = catalog(lang)
  const [listing, setListing] = useState<DirListing | null>(null)
  const [drives, setDrives] = useState<string[]>([])
  const [path, setPath] = useState('')
  const [error, setError] = useState('')

  useEffect(() => {
    if (!props.open) return
    api
      .fsDrives()
      .then((list) => {
        setDrives(list)
        const first = list[0]
        if (first) return api.fsList(first)
        return null
      })
      .then((list) => list && setListing(list))
      .catch(() => setError(t.picker_unreadable))
  }, [props.open, t])

  if (!props.open) return null

  const enter = (target: string) => {
    setError('')
    api
      .fsList(target)
      .then(setListing)
      .catch(() => setError(t.picker_unreadable))
  }

  return (
    <div className="modal-veil" onClick={props.onClose}>
      <div className="modal" onClick={(event) => event.stopPropagation()}>
        <div className="modal-title">{t.picker_title}</div>
        <div className="picker-drives">
          {drives.map((drive) => (
            <button key={drive} className="chip" onClick={() => enter(drive)}>
              {drive}
            </button>
          ))}
        </div>
        <div className="picker-bar">
          <input
            className="text-input"
            value={path}
            placeholder={listing?.path ?? ''}
            onChange={(event) => setPath(event.target.value)}
            onKeyDown={(event) => {
              if (event.key === 'Enter' && path.trim()) enter(path.trim())
            }}
          />
          <button className="button" onClick={() => path.trim() && enter(path.trim())}>
            {t.picker_go}
          </button>
        </div>
        {listing?.parent && (
          <button className="picker-row" onClick={() => enter(listing.parent!)}>
            ..
          </button>
        )}
        <div className="picker-list">
          {listing?.entries.map((entry) => (
            <button key={entry.path} className="picker-row" onClick={() => enter(entry.path)}>
              {entry.name}
            </button>
          ))}
        </div>
        {error && <div className="picker-error">{error}</div>}
        <div className="modal-actions">
          <button className="button" onClick={props.onClose}>
            {t.picker_cancel}
          </button>
          <button
            className="button primary"
            disabled={!listing}
            onClick={() => {
              if (listing) {
                props.onPick(listing.path)
                props.onClose()
              }
            }}
          >
            {t.picker_choose}
          </button>
        </div>
      </div>
    </div>
  )
}
