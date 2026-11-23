import { useEffect, useState } from 'react'
import { RefreshCw } from 'lucide-react'
import { request } from '@/lib/api'
import { Dialog, DialogContent, DialogDescription, DialogFooter, DialogHeader, DialogTitle } from '@/components/ui/dialog'
import { Alert, AlertDescription, AlertTitle } from '@/components/ui/alert'
import { Button } from '@/components/ui/button'

type Logs = { logs: string; truncated: boolean; checked_at: string }

export function SwarmLogsDialog({ service, onClose }: { service: { id: string; name: string | null }; onClose: () => void }) {
  const [data, setData] = useState<Logs | null>(null)
  const [error, setError] = useState('')
  const [refresh, setRefresh] = useState(0)
  useEffect(() => {
    const controller = new AbortController()
    setData(null); setError('')
    request<Logs>(`/docker/swarm/services/${encodeURIComponent(service.id)}/logs`, { signal: AbortSignal.any([controller.signal, AbortSignal.timeout(8000)]) })
      .then(result => { if (!controller.signal.aborted) setData(result) })
      .catch((reason: unknown) => { if (!controller.signal.aborted) setError(reason instanceof Error ? reason.message : 'Could not read service logs.') })
    return () => controller.abort()
  }, [service.id, refresh])
  const text = data?.logs.replace(/\u001b\[[0-?]*[ -/]*[@-~]/g, '') ?? ''
  return <Dialog open onOpenChange={open => { if (!open) onClose() }}><DialogContent className="max-h-[90dvh] min-w-0 overflow-y-auto sm:max-w-4xl">
    <DialogHeader><DialogTitle>Logs: {service.name || service.id}</DialogTitle><DialogDescription>Recent stdout and stderr across this service’s tasks, requested up to 200 lines per task with timestamps. Snapshot limited to 64 KiB. Logs may contain application secrets; share carefully.</DialogDescription></DialogHeader>
    {error && <Alert variant="destructive"><AlertTitle>Service logs unavailable</AlertTitle><AlertDescription>{error}</AlertDescription></Alert>}
    {!data && !error && <p role="status" className="text-sm text-muted-foreground">Loading service logs…</p>}
    {data && <>
      {data.truncated && <Alert><AlertTitle>Log snapshot truncated</AlertTitle><AlertDescription>Only part of the available logs is shown because the read or display limit was reached.</AlertDescription></Alert>}
      {text ? <pre tabIndex={0} aria-label="Swarm service logs" className="max-h-[50dvh] min-w-0 overflow-auto rounded-lg border bg-muted p-4 font-mono text-xs whitespace-pre-wrap break-all">{text}</pre> : <p role="status" className="py-6 text-sm text-muted-foreground">No logs reported by Docker for this service.</p>}
      <p className="text-xs text-muted-foreground">Snapshot checked {new Date(data.checked_at).toLocaleString()}. Refresh to read new entries.</p>
    </>}
    <DialogFooter><Button variant="outline" disabled={!data && !error} onClick={() => setRefresh(value => value + 1)}><RefreshCw aria-hidden="true" />Refresh logs</Button><Button variant="outline" onClick={onClose}>Close logs</Button></DialogFooter>
  </DialogContent></Dialog>
}
