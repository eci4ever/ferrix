import { useEffect, useState } from 'react'
import { request } from '@/lib/api'
import { DataTable } from './data-table'
import type { DataColumn } from './data-table'
import { Alert, AlertDescription, AlertTitle } from './ui/alert'
import { Button } from './ui/button'
import { NativeSelect, NativeSelectOption } from './ui/native-select'
export function PagedTable<T extends { id: string }>({ path, label, columns, refreshVersion, roles = false, statuses = false, defaultStatus = '' }: { path: string; label: string; columns: DataColumn<T>[]; refreshVersion: number; roles?: boolean; statuses?: boolean; defaultStatus?: string }) {
  const [search, setSearch] = useState(''), [page, setPage] = useState(0), [role, setRole] = useState(''), [status, setStatus] = useState(defaultStatus)
  const [result, setResult] = useState<{ items: T[]; total: number }>({ items: [], total: 0 }), [error, setError] = useState(''), [loading, setLoading] = useState(true), [retry, setRetry] = useState(0)
  useEffect(() => {
    const abort = new AbortController(); setLoading(true); setError('')
    const timer = setTimeout(() => {
      const query = new URLSearchParams({ q: search, page: String(page), role, status })
      request<{ items: T[]; total: number } | T[]>(`${path}?${query}`, { signal: abort.signal }).then(data => {
        if (abort.signal.aborted) return
        const value = Array.isArray(data) ? { items: data, total: data.length } : data
        if (!Array.isArray(value.items) || typeof value.total !== 'number') throw new Error('Invalid list response. Please retry.')
        if (page && page * 25 >= value.total) { setPage(Math.max(0, Math.ceil(value.total / 25) - 1)); return }
        setResult(value); setLoading(false)
      }).catch(reason => { if (!abort.signal.aborted) { setError(reason.message); setResult({ items: [], total: 0 }); setLoading(false) } })
    }, 250)
    return () => { clearTimeout(timer); abort.abort() }
  }, [path, search, page, role, status, refreshVersion, retry])
  return <div className="min-w-0 space-y-4">
    {error ? <Alert variant="destructive"><AlertTitle>{label} could not be loaded</AlertTitle><AlertDescription>{error}<Button variant="outline" onClick={() => setRetry(value => value + 1)}>Retry {label.toLowerCase()}</Button></AlertDescription></Alert> : null}
    <DataTable label={label} columns={columns} data={result.items} loading={loading} server={{ total: result.total, pageIndex: page, pageSize: 25, search, onSearchChange: value => { setSearch(value.slice(0, 120)); setPage(0) }, onPageChange: setPage }} toolbar={roles || statuses ? <div className="flex flex-wrap gap-2">
      {roles ? <NativeSelect aria-label={`${label} role`} value={role} onChange={event => { setRole(event.target.value); setPage(0) }}><NativeSelectOption value="">All roles</NativeSelectOption>{['owner', 'admin', 'member'].map(value => <NativeSelectOption key={value} value={value}>{value}</NativeSelectOption>)}</NativeSelect> : null}
      {statuses ? <NativeSelect aria-label={`${label} status`} value={status} onChange={event => { setStatus(event.target.value); setPage(0) }}><NativeSelectOption value="">All statuses</NativeSelectOption>{['pending', 'accepted', 'expired', 'cancelled', 'rejected'].map(value => <NativeSelectOption key={value} value={value}>{value}</NativeSelectOption>)}</NativeSelect> : null}
    </div> : null} />
  </div>
}
