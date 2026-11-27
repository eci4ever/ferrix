import { DataTableFilter } from '@/components/data-table-filter'
import { useEffect, useState } from 'react'
import { RefreshCw, X } from 'lucide-react'
import { DataTable } from '@/components/data-table'
import { Alert, AlertDescription, AlertTitle } from '@/components/ui/alert'
import { Badge } from '@/components/ui/badge'
import { Button } from '@/components/ui/button'
import { Card, CardContent, CardDescription, CardHeader, CardTitle } from '@/components/ui/card'
import { Tabs, TabsContent, TabsList, TabsTrigger } from '@/components/ui/tabs'
import { request } from '@/lib/api'
import { SwarmInventory } from '@/swarm'

const sections = [
  { id: 'containers', label: 'Containers', description: 'Running and stopped containers on this Docker host.', columns: [['name', 'Name'], ['state', 'State'], ['status', 'Status'], ['image', 'Image']] },
  { id: 'swarm', label: 'Swarm', description: 'Cluster membership and manager status for this Docker node.', columns: [['state', 'Local node state'], ['manager', 'Manager access'], ['nodes', 'Nodes'], ['managers', 'Managers']] },
  { id: 'images', label: 'Images', description: 'Image tags and sizes on this Docker host.', columns: [['tags', 'Tags'], ['id', 'Image ID'], ['size', 'Size']] },
  { id: 'volumes', label: 'Volumes', description: 'Named volumes and their storage drivers.', columns: [['id', 'Name'], ['driver', 'Driver'], ['scope', 'Scope']] },
  { id: 'networks', label: 'Networks', description: 'Docker networks and their drivers.', columns: [['name', 'Name'], ['driver', 'Driver'], ['scope', 'Scope']] },
  { id: 'events', label: 'Events', description: 'Up to 256 events retained by Docker in the last 15 minutes. Refresh to get a new snapshot.', columns: [['time', 'Time'], ['type', 'Type'], ['action', 'Action'], ['object', 'Object ID']] },
  { id: 'disk-usage', label: 'Disk Usage', description: 'Docker storage estimates sampled once a minute. Categories can share disk space.', columns: [['images', 'Images'], ['containers', 'Containers'], ['volumes', 'Volumes'], ['build_cache', 'Build cache'], ['at', 'Last sample']] },
  { id: 'health', label: 'Health', description: 'Docker daemon connectivity and engine version.', columns: [['connected', 'Connected'], ['version', 'Engine version'], ['api', 'API version'], ['os', 'OS'], ['arch', 'Architecture']] },
]

type Section = typeof sections[number]
type DockerRecord = { id: string } & Record<string, string | number | boolean | null | undefined>

function formatValue(key: string, value: DockerRecord[string]) {
  if (value == null || value === '') return '—'
  if (typeof value === 'boolean') return value ? 'Yes' : 'No'
  if (typeof value !== 'number') return value
  if (key === 'time') return new Date(value * 1000).toLocaleString()
  if (key === 'at') return new Date(value).toLocaleString()
  if (['size', 'images', 'containers', 'volumes', 'build_cache'].includes(key)) {
    return new Intl.NumberFormat(undefined, { maximumFractionDigits: 1 }).format(value / 1048576) + ' MiB'
  }
  return String(value)
}

function Inventory({ section, refreshVersion }: { section: Section; refreshVersion: number }) {
  const [data, setData] = useState<DockerRecord | DockerRecord[] | null>(null)
  const [error, setError] = useState('')
  const [loading, setLoading] = useState(true)
  const [refresh, setRefresh] = useState(0)
  const [state, setState] = useState('')
  useEffect(() => {
    const controller = new AbortController()
    setError('')
    setLoading(true)
    const path = section.id === 'containers' ? '/containers' : '/docker/' + section.id
    request<DockerRecord | DockerRecord[]>(path, { signal: AbortSignal.any([controller.signal, AbortSignal.timeout(8000)]) })
      .then(result => { if (!controller.signal.aborted) setData(result) })
      .catch((reason: unknown) => {
        if (!controller.signal.aborted) setError(reason instanceof Error ? reason.message : 'Docker inventory unavailable.')
      }).finally(() => { if (!controller.signal.aborted) setLoading(false) })
    return () => controller.abort()
  }, [section.id, refresh, refreshVersion])

  const rows = Array.isArray(data) ? data : []
  const states = Array.from(new Set(rows.map(row => String(row.state || 'unknown')))).sort()
  const filtered = state ? rows.filter(row => String(row.state || 'unknown') === state) : rows
  return <Card className="min-w-0">
    <CardHeader>
      <CardTitle><h2>{section.label}</h2></CardTitle>
      <CardDescription>{section.description}</CardDescription>
    </CardHeader>
    <CardContent className="flex min-w-0 flex-col gap-4">
      <div className="flex flex-wrap items-center gap-2">
        <Button variant="outline" onClick={() => setRefresh(previous => previous + 1)} disabled={loading}>
          <RefreshCw aria-hidden="true" /> Refresh {section.label.toLowerCase()}
        </Button>

      </div>
      {error && <Alert variant="destructive" role="alert"><AlertTitle>{section.label} unavailable</AlertTitle><AlertDescription>{error} Use Refresh to retry.</AlertDescription></Alert>}
      {!data && !error && <p role="status" className="text-sm text-muted-foreground">Loading {section.label.toLowerCase()}…</p>}
      {Array.isArray(data) && <DataTable<DockerRecord> label={section.label} loading={loading} data={filtered} filterKey={state} toolbar={section.id === 'containers' ? <div className="flex flex-wrap items-center gap-2" role="group" aria-label="Container filters"><DataTableFilter label="Status" value={state} onChange={setState} options={[[ '', 'All states' ], ...Array.from(new Set([...states, ...(state ? [state] : [])])).map(value => [value, value] as [string, string])]} />{state ? <Button variant="ghost" size="sm" className="h-8" onClick={() => setState('')}><X aria-hidden="true" />Reset filters</Button> : null}</div> : undefined} columns={section.columns.map(([key, label]) => ({
        id: key, label, value: row => formatValue(key, row[key]),
        cell: row => key === 'state' ? <Badge variant="secondary">{formatValue(key, row[key])}</Badge> : <span className="block min-w-24 max-w-80 whitespace-normal break-all">{formatValue(key, row[key])}</span>,
      }))} />}
      {data && !Array.isArray(data) && <>
        {data.error && <Alert role="status"><AlertTitle>Storage sample unavailable</AlertTitle><AlertDescription>{String(data.error)}</AlertDescription></Alert>}
        {section.id === 'disk-usage' && !data.at && !data.error && <p className="text-sm text-muted-foreground">Waiting for the first storage sample. Refresh in a minute.</p>}
        <dl className="grid gap-3 sm:grid-cols-2 lg:grid-cols-3">
          {section.columns.map(([key, label]) => <div key={key} className="rounded-lg border p-3">
            <dt className="text-sm text-muted-foreground">{label}</dt>
            <dd className="mt-1 break-words font-medium">{formatValue(key, data[key])}</dd>
          </div>)}
        </dl>
      </>}
    </CardContent>
  </Card>
}

export function DockerPage({ admin, refreshVersion }: { admin: boolean; refreshVersion: number }) {

  return <section className="flex min-w-0 flex-col gap-4">
    <div><h1 className="text-2xl font-semibold tracking-tight">Docker</h1><p className="text-sm text-muted-foreground">Inventory and diagnostics for the connected Docker host.</p></div>
    <Tabs defaultValue="containers" className="min-w-0">
      <div className="overflow-x-auto p-1">
        <TabsList variant="line" aria-label="Docker sections">
          {sections.map(section => <TabsTrigger key={section.id} value={section.id}>{section.label}</TabsTrigger>)}
        </TabsList>
      </div>
      {sections.map(section => <TabsContent key={section.id} value={section.id} className="min-w-0">
        {section.id === 'swarm' ? <SwarmInventory refreshVersion={refreshVersion} canManage={admin} /> : <Inventory section={section} refreshVersion={refreshVersion} />}
      </TabsContent>)}
    </Tabs>
  </section>
}
