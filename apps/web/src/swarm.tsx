import { useEffect, useState } from 'react'
import { Activity, Network, RefreshCw, Server } from 'lucide-react'
import { Alert, AlertDescription, AlertTitle } from '@/components/ui/alert'
import { Badge } from '@/components/ui/badge'
import { Button } from '@/components/ui/button'
import { Card, CardContent, CardDescription, CardHeader, CardTitle } from '@/components/ui/card'
import { Tabs, TabsContent, TabsList, TabsTrigger } from '@/components/ui/tabs'
import { request } from '@/lib/api'

type SwarmNode = {
  id: string | null; hostname: string | null; role: string | null
  availability: string | null; state: string | null; address: string | null
  leader: boolean | null; reachability: string | null; engine_version: string | null
  nano_cpus: number | null; memory_bytes: number | null
}
type Swarm = {
  state: string | null; manager: boolean | null; nodes: number | null; managers: number | null
  node_id: string | null; node_address: string | null; node_list: SwarmNode[] | null
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
      {error && <Alert variant="destructive"><AlertTitle>Swarm unavailable</AlertTitle><AlertDescription>{error} Use Refresh swarm to retry.</AlertDescription></Alert>}
      {!data && !error && <p role="status" className="text-sm text-muted-foreground">Loading swarm…</p>}
      {data && <>
        <div className="flex flex-wrap items-center gap-2 text-sm"><Badge variant={active ? 'default' : 'secondary'}>{data.state || 'unknown'}</Badge><span className="text-muted-foreground">Local node{data.manager ? ' · Manager access' : ''}</span></div>
        {!active && <Alert><AlertTitle>{data.state === 'inactive' ? 'Swarm is not active' : 'Swarm is not ready'}</AlertTitle><AlertDescription>This Docker host {data.state === 'inactive' ? 'has not joined a Swarm cluster' : 'cannot currently provide an active cluster view'}. Cluster counts and nodes are unavailable. Refresh after Swarm is active; Ferrix does not change cluster configuration.</AlertDescription></Alert>}
        {active && !data.manager && <Alert><AlertTitle>Manager access required</AlertTitle><AlertDescription>This host is a Swarm worker. Connect Ferrix to a manager to view cluster counts and nodes. Local node: {data.node_id || '—'} · {data.node_address || '—'}.</AlertDescription></Alert>}
        <Tabs defaultValue="overview" className="min-w-0">
          <TabsList aria-label="Swarm views"><TabsTrigger value="overview">Overview</TabsTrigger><TabsTrigger value="nodes">Nodes</TabsTrigger></TabsList>
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
          <TabsContent value="nodes">
            {nodes ? nodes.length ? <div className="grid gap-4 lg:grid-cols-2 xl:grid-cols-3">{nodes.map((node, index) => <NodeCard key={node.id || index} node={node} />)}</div> : <p className="py-4 text-sm text-muted-foreground">Docker returned no nodes. Refresh to check the cluster again.</p> : <p className="py-4 text-sm text-muted-foreground">Node inventory is unavailable until this host is an active Swarm manager.</p>}
          </TabsContent>
        </Tabs>
      </>}
    </CardContent>
  </Card>
}
