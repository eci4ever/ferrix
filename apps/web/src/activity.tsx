import { useEffect, useState } from 'react'
import type { Identity } from './auth'
import { request } from '@/lib/api'
import { DataTable } from '@/components/data-table'
import { Alert, AlertTitle, AlertDescription } from '@/components/ui/alert'
import { Button } from '@/components/ui/button'
import { Field, FieldLabel } from '@/components/ui/field'
import { NativeSelect, NativeSelectOption } from '@/components/ui/native-select'
import { Badge } from '@/components/ui/badge'

type Option = { id: string; name: string }
type Event = { id: string; user_id: string | null; user_name: string; organization_id: string | null; organization_name: string; action: string; target_id: string | null; outcome: string; created_at: string }
type Result = { items: Event[]; total: number; users: Option[]; organizations: Option[] }
export function ActivityPage({ identity, refreshVersion, organizationId }: { identity: Identity; refreshVersion: number; organizationId?: string }) {
  const [search, setSearch] = useState(''), [organization, setOrganization] = useState(''), [user, setUser] = useState(''), [page, setPage] = useState(0)
  const [result, setResult] = useState<Result | null>(null), [error, setError] = useState(''), [loading, setLoading] = useState(true), [retry, setRetry] = useState(0)
  useEffect(() => {
    const abort = new AbortController(); setLoading(true); setError('')
    const timer = setTimeout(() => {
      const query = new URLSearchParams({ q: search, page: String(page) })
      if (organizationId || organization) query.set('organization_id', organizationId || organization)
      if (user) query.set('user_id', user)
      request<Result>(`/auth/activity?${query}`, { signal: abort.signal }).then(value => {
        if (abort.signal.aborted) return
        if (page && page * 25 >= value.total) { setPage(Math.max(0, Math.ceil(value.total / 25) - 1)); return }
        setResult(value); setLoading(false)
      }).catch(reason => { if (!abort.signal.aborted) { setError(reason.message); setLoading(false) } })
    }, 250)
    return () => { clearTimeout(timer); abort.abort() }
  }, [search, organization, user, page, refreshVersion, retry, identity.organization_id, organizationId])
  const platform = identity.system_role === 'admin'
  const manager = identity.role === 'owner' || identity.role === 'admin'
  return <div className="flex min-w-0 flex-col gap-6">
    <div className="space-y-1">{organizationId ? <h2 className="text-xl font-semibold">Activity</h2> : <h1 className="text-2xl font-semibold">Activity</h1>}<p className="text-sm text-muted-foreground">{organizationId ? 'Activity in this organization.' : platform ? 'Activity across all organizations.' : manager ? 'Activity in your active organization.' : 'Your activity in your active organization.'} Pending events have an unconfirmed outcome; accepted operations may still run in the background.</p></div>
    {error ? <Alert variant="destructive"><AlertTitle>Activity could not be loaded</AlertTitle><AlertDescription>{error}<Button variant="outline" onClick={() => setRetry(value => value + 1)}>Retry activity</Button></AlertDescription></Alert> : null}
    <div className="flex flex-wrap gap-4">
      {platform && !organizationId ? <Field className="w-full sm:w-64"><FieldLabel htmlFor="activity-org">Organization</FieldLabel><NativeSelect id="activity-org" value={organization} onChange={event => { setOrganization(event.target.value); setPage(0) }}><NativeSelectOption value="">All organizations</NativeSelectOption>{(result?.organizations ?? []).map(option => <NativeSelectOption key={option.id} value={option.id}>{option.name}</NativeSelectOption>)}</NativeSelect></Field> : null}
      {platform || manager ? <Field className="w-full sm:w-64"><FieldLabel htmlFor="activity-user">User</FieldLabel><NativeSelect id="activity-user" value={user} onChange={event => { setUser(event.target.value); setPage(0) }}><NativeSelectOption value="">All users</NativeSelectOption>{(result?.users ?? []).map(option => <NativeSelectOption key={option.id} value={option.id}>{option.name}</NativeSelectOption>)}</NativeSelect></Field> : null}
      <Button variant="outline" disabled={!search && !organization && !user} onClick={() => { setSearch(''); setOrganization(''); setUser(''); setPage(0) }}>Clear filters</Button>
    </div>
    <DataTable label="Activity" loading={loading} data={error ? [] : result?.items ?? []} server={{ total: result?.total ?? 0, pageIndex: page, pageSize: 25, search, onSearchChange: value => { setSearch(value.slice(0, 120)); setPage(0) }, onPageChange: setPage }} columns={[
      { id: 'time', label: 'Time', cell: item => <time dateTime={item.created_at}>{new Date(item.created_at).toLocaleString()}</time> },
      { id: 'user', label: 'User', cell: item => item.user_name },
      { id: 'organization', label: 'Organization', cell: item => item.organization_name },
      { id: 'action', label: 'Action', cell: item => <span className="whitespace-normal break-words">{item.action}</span> },
      { id: 'target', label: 'Resource ID', cell: item => item.target_id ?? '—' },
      { id: 'outcome', label: 'Outcome', cell: item => <Badge variant={item.outcome === 'failed' ? 'destructive' : 'outline'}>{item.outcome}</Badge> },
    ]} />
  </div>
}
