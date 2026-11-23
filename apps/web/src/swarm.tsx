import { useEffect, useState } from 'react'
import { Activity, Network, RefreshCw, Server } from 'lucide-react'
import { Alert, AlertDescription, AlertTitle } from '@/components/ui/alert'
import { Badge } from '@/components/ui/badge'
import { Button } from '@/components/ui/button'
import { Card, CardContent, CardDescription, CardHeader, CardTitle } from '@/components/ui/card'
import { Tabs, TabsContent, TabsList, TabsTrigger } from '@/components/ui/tabs'
import { request } from '@/lib/api'
import { DataTable } from '@/components/data-table'
import { SwarmScaleDialog } from '@/swarm-scale-dialog'
import { SwarmRestartDialog } from '@/swarm-restart-dialog'
import { SwarmLogsDialog } from '@/swarm-logs-dialog'
import { ActionNotice } from '@/components/action-notice'

type SwarmService = {
  id: string; name: string | null; image: string | null; mode: string | null
  running_tasks: number | null; desired_tasks: number | null; completed_tasks: number | null
  update_state: string | null
  replicas: number | null; version: number | null
  ports: { published: number | null; target: number | null; protocol: string | null; mode: string | null }[] | null
}

function modeLabel(mode: string | null) {
  return ({ Replicated: 'Replicated', Global: 'Global', ReplicatedJob: 'Replicated job', GlobalJob: 'Global job' })[mode ?? ''] || 'Unknown'
}

function portLabels(service: SwarmService) {
  return service.ports?.map(port => `${port.published ?? '—'} → ${port.target ?? '—'}/${port.protocol ?? 'unknown'} (${port.mode ?? 'unknown'})`).join(', ') || 'None reported'
}

function SwarmServices({ services, active, manager, onScale, onRestart, onLogs }: { services: SwarmService[] | null | undefined; active: boolean; manager: boolean; onScale: (service: SwarmService) => void; onRestart: (service: SwarmService) => void; onLogs: (service: SwarmService) => void }) {
  if (!active) return <Alert><AlertTitle>Services unavailable: Swarm is not active</AlertTitle><AlertDescription>Activate or join a Swarm cluster outside Ferrix, then refresh to view its services.</AlertDescription></Alert>
  if (!manager) return <Alert><AlertTitle>Services require manager access</AlertTitle><AlertDescription>The connected Docker node cannot list cluster services. Connect Ferrix to an active Swarm manager and refresh.</AlertDescription></Alert>
  if (!services) return <Alert><AlertTitle>Service inventory unavailable</AlertTitle><AlertDescription>Docker did not return a service inventory. Refresh swarm to retry.</AlertDescription></Alert>
  if (!services.length) return <Alert><AlertTitle>No Swarm services</AlertTitle><AlertDescription>This manager reported no services in the cluster.</AlertDescription></Alert>
  return <div className="min-w-0 space-y-4">
    <p className="text-sm text-muted-foreground">{services.length} cluster {services.length === 1 ? 'service' : 'services'}. Tasks show running / desired counts reported by Docker. Completed tasks are shown separately for jobs.</p>
    <DataTable<SwarmService> label="Swarm services" data={services} columns={[
      { id: 'name', label: 'Service', value: service => service.name || service.id, cell: service => <div className="min-w-36 max-w-64 whitespace-normal break-all"><p className="font-medium">{service.name || service.id}</p><p className="text-xs text-muted-foreground">{service.id}</p></div> },
      { id: 'mode', label: 'Mode', value: service => modeLabel(service.mode), cell: service => <Badge variant="secondary">{modeLabel(service.mode)}</Badge> },
      { id: 'tasks', label: 'Tasks', value: service => `${service.running_tasks ?? '—'} / ${service.desired_tasks ?? '—'}`, cell: service => <div className="min-w-28 tabular-nums"><p>{service.running_tasks ?? '—'} / {service.desired_tasks ?? '—'}</p>{service.mode?.endsWith('Job') && <p className="text-xs text-muted-foreground">{service.completed_tasks ?? '—'} completed</p>}</div> },
      { id: 'image', label: 'Image', value: service => service.image || '—', cell: service => <span className="block min-w-40 max-w-80 whitespace-normal break-all">{service.image || '—'}</span> },
      { id: 'ports', label: 'Published ports', value: portLabels, cell: service => <span className="block min-w-36 max-w-64 whitespace-normal break-words">{portLabels(service)}</span> },
      { id: 'update', label: 'Update', value: service => service.update_state || 'No update reported', cell: service => service.update_state ? <Badge variant={['paused', 'rollback_paused'].includes(service.update_state) ? 'destructive' : 'outline'}>{service.update_state.replaceAll('_', ' ')}</Badge> : <span className="text-muted-foreground">No update reported</span> },
      { id: 'scale', label: 'Replicas', cell: service => service.mode === 'Replicated' ? <Button variant="outline" size="sm" disabled={service.replicas == null || service.version == null || service.version < 1 || service.version > 2147483647} aria-label={`Scale ${service.name || service.id}`} onClick={() => onScale(service)}>Scale replicas</Button> : <span className="text-xs text-muted-foreground">{service.mode?.endsWith('Job') ? 'Job mode' : service.mode === 'Global' ? 'Managed by node count' : 'Scale unavailable'}</span> },
      { id: 'restart', label: 'Restart', cell: service => ['Replicated', 'Global'].includes(service.mode ?? '') ? <Button variant="outline" size="sm" disabled={service.version == null || service.version < 1 || service.version > 2147483647} aria-label={`Restart ${service.name || service.id}`} onClick={() => onRestart(service)}>Restart</Button> : <span className="text-xs text-muted-foreground">Restart unavailable</span> },
      { id: 'logs', label: 'Logs', cell: service => <Button variant="outline" size="sm" aria-label={`View logs for ${service.name || service.id}`} onClick={() => onLogs(service)}>View logs</Button> },
    ]} />
  </div>
}

type SwarmNode = {
  id: string | null; hostname: string | null; role: string | null
  availability: string | null; state: string | null; address: string | null
  leader: boolean | null; reachability: string | null; engine_version: string | null
  nano_cpus: number | null; memory_bytes: number | null
}
type Swarm = {
  state: string | null; manager: boolean | null; nodes: number | null; managers: number | null
  node_id: string | null; node_address: string | null; node_list: SwarmNode[] | null
  service_list: SwarmService[] | null
}

function NodeCard({ node }: { node: SwarmNode }) {
  return <Card className="min-w-0">
    <CardHeader>
      <CardTitle className="flex flex-wrap items-center gap-2">
        <span className={`size-2 shrink-0 rounded-full ${node.state === 'ready' ? 'bg-status-success' : node.state === 'down' ? 'bg-destructive' : 'bg-muted-foreground'}`} aria-hidden="true" />
        <h3 className="break-all">{node.hostname || node.id || 'Unnamed node'}</h3>
        <Badge variant="secondary">{node.leader ? 'Leader' : node.role || 'Unknown role'}</Badge>
      </CardTitle>
      <CardDescription className="flex flex-wrap gap-2">
        <Badge variant="outline">Status: {node.state || 'unknown'}</Badge>
        <Badge variant="outline">Availability: {node.availability || 'unknown'}</Badge>
        {node.role === 'manager' && <Badge variant="outline">Manager: {node.reachability || 'unknown'}</Badge>}
      </CardDescription>
    </CardHeader>
    <CardContent>
      <dl className="grid grid-cols-2 gap-4 text-sm">
        {[
          ['Engine version', node.engine_version],
          ['CPU capacity', node.nano_cpus == null ? null : `${node.nano_cpus / 1e9} cores`],
          ['Memory capacity', node.memory_bytes == null ? null : `${(node.memory_bytes / 1073741824).toLocaleString(undefined, { maximumFractionDigits: 2 })} GiB`],
          ['IP address', node.address],
          ['Node ID', node.id],
        ].map(([label, value]) => <div key={label}><dt className="text-muted-foreground">{label}</dt><dd className="mt-1 break-all font-medium">{value || '—'}</dd></div>)}
      </dl>
    </CardContent>
  </Card>
}

export function SwarmInventory({ refreshVersion }: { refreshVersion: number }) {
  const [data, setData] = useState<Swarm | null>(null)
  const [error, setError] = useState('')
  const [refresh, setRefresh] = useState(0)
  const [view, setView] = useState('overview')
  const [scaling, setScaling] = useState<SwarmService | null>(null)
  const [restarting, setRestarting] = useState<SwarmService | null>(null)
  const [logging, setLogging] = useState<SwarmService | null>(null)
  const [notice, setNotice] = useState('')
  useEffect(() => {
    const controller = new AbortController()
    setData(null)
    setError('')
    request<Swarm>('/docker/swarm', { signal: AbortSignal.any([controller.signal, AbortSignal.timeout(8000)]) })
      .then(result => { if (!controller.signal.aborted) setData(result) })
      .catch((reason: unknown) => { if (!controller.signal.aborted) setError(reason instanceof Error ? reason.message : 'Docker inventory unavailable.') })
    return () => controller.abort()
  }, [refresh, refreshVersion])
  const nodes = data?.node_list
  const active = data?.state === 'active'
  return <Card className="min-w-0">
    <CardHeader className="gap-3 sm:flex-row sm:items-start sm:justify-between">
      <div className="space-y-1.5"><CardTitle><h2 className="flex items-center gap-2"><Network aria-hidden="true" className="size-5" />Docker Swarm</h2></CardTitle><CardDescription>Cluster overview and node capacity from the connected Docker host.</CardDescription></div>
      <Button variant="outline" disabled={!data && !error} onClick={() => setRefresh(value => value + 1)}><RefreshCw aria-hidden="true" />Refresh swarm</Button>
    </CardHeader>
    <CardContent className="min-w-0 space-y-4">
      {notice && <ActionNotice message={notice} onDismiss={() => setNotice('')} />}
      {error && <Alert variant="destructive"><AlertTitle>Swarm unavailable</AlertTitle><AlertDescription>{error} Use Refresh swarm to retry.</AlertDescription></Alert>}
      {!data && !error && <p role="status" className="text-sm text-muted-foreground">Loading swarm…</p>}
      {data && <>
        <div className="flex flex-wrap items-center gap-2 text-sm"><Badge variant={active ? 'default' : 'secondary'}>{data.state || 'unknown'}</Badge><span className="text-muted-foreground">Local node{data.manager ? ' · Manager access' : ''}</span></div>
        {!active && <Alert><AlertTitle>{data.state === 'inactive' ? 'Swarm is not active' : 'Swarm is not ready'}</AlertTitle><AlertDescription>This Docker host {data.state === 'inactive' ? 'has not joined a Swarm cluster' : 'cannot currently provide an active cluster view'}. Cluster counts, nodes and services are unavailable. Refresh after Swarm is active; Ferrix does not change cluster configuration.</AlertDescription></Alert>}
        {active && !data.manager && <Alert><AlertTitle>Manager access required</AlertTitle><AlertDescription>This host is a Swarm worker. Connect Ferrix to a manager to view cluster counts, nodes and services. Local node: {data.node_id || '—'} · {data.node_address || '—'}.</AlertDescription></Alert>}
        <Tabs value={view} onValueChange={setView} className="min-w-0">
          <TabsList aria-label="Swarm views"><TabsTrigger value="overview">Overview</TabsTrigger><TabsTrigger value="nodes">Nodes</TabsTrigger><TabsTrigger value="services">Services</TabsTrigger></TabsList>
          <TabsContent value="overview" className="space-y-4">
            <div className="grid gap-4 md:grid-cols-3">
              {[
                { label: 'Total nodes', value: nodes ? nodes.length : null, icon: Server },
                { label: 'Ready nodes', value: nodes ? `${nodes.filter(node => node.state === 'ready').length} / ${nodes.length}` : null, icon: Activity },
                { label: 'Manager nodes', value: nodes ? nodes.filter(node => node.role === 'manager').length : null, icon: Network },
              ].map(({ label, value, icon: Icon }) => <Card key={label}><CardHeader><CardTitle className="flex items-center justify-between text-sm"><h3>{label}</h3><Icon className="size-4 text-muted-foreground" aria-hidden="true" /></CardTitle></CardHeader><CardContent><p className="text-3xl font-semibold tabular-nums">{value ?? '—'}</p></CardContent></Card>)}
            </div>
            <div className="grid gap-4 lg:grid-cols-2 xl:grid-cols-3">{nodes?.map((node, index) => <NodeCard key={node.id || index} node={node} />)}</div>
          </TabsContent>
          <TabsContent value="services" className="min-w-0"><SwarmServices services={data.service_list} active={active} manager={data.manager === true} onScale={setScaling} onRestart={setRestarting} onLogs={setLogging} /></TabsContent>
          <TabsContent value="nodes">
            {nodes ? nodes.length ? <div className="grid gap-4 lg:grid-cols-2 xl:grid-cols-3">{nodes.map((node, index) => <NodeCard key={node.id || index} node={node} />)}</div> : <p className="py-4 text-sm text-muted-foreground">Docker returned no nodes. Refresh to check the cluster again.</p> : <p className="py-4 text-sm text-muted-foreground">Node inventory is unavailable until this host is an active Swarm manager.</p>}
          </TabsContent>
        </Tabs>
      </>}
    </CardContent>
    {scaling && <SwarmScaleDialog service={scaling} onClose={() => setScaling(null)} onScaled={replicas => { setNotice(`Scale requested for ${scaling.name || scaling.id}: ${replicas} replicas. Docker may take a moment to converge; refresh swarm to check task counts.`); setScaling(null); setRefresh(value => value + 1) }} />}
    {restarting && <SwarmRestartDialog service={restarting} onClose={() => setRestarting(null)} onRestarted={() => { setNotice(`Restart requested for ${restarting.name || restarting.id}. Docker is replacing tasks; refresh swarm to check the running/desired counts.`); setRestarting(null); setRefresh(value => value + 1) }} />}
    {logging && <SwarmLogsDialog service={logging} onClose={() => setLogging(null)} />}
  </Card>
}
