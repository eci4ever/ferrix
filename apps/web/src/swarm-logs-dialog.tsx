import { useEffect, useRef, useState } from 'react'
import { Pause, Play, RefreshCw } from 'lucide-react'
import { ApiRequestError, request } from '@/lib/api'
import { Dialog, DialogContent, DialogDescription, DialogFooter, DialogHeader, DialogTitle } from '@/components/ui/dialog'
import { Alert, AlertDescription, AlertTitle } from '@/components/ui/alert'
import { Button } from '@/components/ui/button'

type Logs = { logs: string; truncated: boolean; checked_at: string }

export function SwarmLogsDialog({ service, onClose }: { service: { id: string; name: string | null }; onClose: () => void }) {
  const [data, setData] = useState<Logs | null>(null)
  const [error, setError] = useState('')
  const [refresh, setRefresh] = useState(0)
  const [paused, setPaused] = useState(false)
  const [busy, setBusy] = useState(false)
  const attempted = useRef('')
  useEffect(() => {
    const controller = new AbortController()
    let inFlight = false
    let timer: ReturnType<typeof setTimeout> | undefined
    const source = `${service.id}:${refresh}`
    setBusy(false)
    async function read() {
      if (controller.signal.aborted || document.hidden || inFlight) return
      clearTimeout(timer)
      inFlight = true
      attempted.current = source
      setBusy(true)
      let stop = false
      try {
        const result = await request<Logs>(`/docker/swarm/services/${encodeURIComponent(service.id)}/logs`, { signal: AbortSignal.any([controller.signal, AbortSignal.timeout(8000)]) })
        if (!controller.signal.aborted) { setData(result); setError('') }
      } catch (reason) {
        if (controller.signal.aborted) return
        if (reason instanceof ApiRequestError && [401, 403, 404].includes(reason.status)) {
          setData(null); setPaused(true); stop = true
        }
        setError(reason instanceof Error ? reason.message : 'Could not read service logs.')
      } finally {
        inFlight = false
        if (!controller.signal.aborted) {
          setBusy(false)
          if (!paused && !stop && !document.hidden) timer = setTimeout(() => void read(), 3000)
        }
      }
    }
    if (!paused || attempted.current !== source) void read()
    const visible = () => {
      if (document.hidden) clearTimeout(timer)
      else if (!paused || attempted.current !== source) void read()
    }
    document.addEventListener('visibilitychange', visible)
    return () => { controller.abort(); clearTimeout(timer); document.removeEventListener('visibilitychange', visible) }
  }, [service.id, refresh, paused])
  const text = data?.logs.replace(/\u001b\[[0-?]*[ -/]*[@-~]/g, '') ?? ''
  return <Dialog open onOpenChange={open => { if (!open) onClose() }}><DialogContent className="max-h-[90dvh] min-w-0 overflow-y-auto sm:max-w-4xl">
    <DialogHeader><DialogTitle>Logs: {service.name || service.id}</DialogTitle><DialogDescription>Recent stdout and stderr across this service’s tasks, requested up to 200 lines per task with timestamps. Snapshot limited to 64 KiB. Logs may contain application secrets; share carefully.</DialogDescription></DialogHeader>
    <p role="status" className="text-sm text-muted-foreground">{paused ? 'Auto-refresh paused.' : 'Auto-refresh on · checks again 3 seconds after each request while this tab is visible.'}{busy ? ' Fetching logs…' : ''}</p>
    {error && <Alert variant="destructive"><AlertTitle>Service logs unavailable</AlertTitle><AlertDescription>{error}{data ? ' Showing the last successful snapshot.' : ''}</AlertDescription></Alert>}
    {!data && !error && <p className="text-sm text-muted-foreground">{busy ? 'Loading service logs…' : 'No snapshot loaded. Use Refresh logs.'}</p>}
    {data && <>
      {data.truncated && <Alert><AlertTitle>Log snapshot truncated</AlertTitle><AlertDescription>Only part of the available logs is shown because the read or display limit was reached.</AlertDescription></Alert>}
      {text ? <pre tabIndex={0} aria-label="Swarm service logs" className="max-h-[50dvh] min-w-0 overflow-auto rounded-lg border bg-muted p-4 font-mono text-xs whitespace-pre-wrap break-all">{text}</pre> : <p role="status" className="py-6 text-sm text-muted-foreground">No logs reported by Docker for this service.</p>}
      <p className="text-xs text-muted-foreground">Snapshot checked {new Date(data.checked_at).toLocaleString()}.{paused ? ' Resume auto-refresh or refresh manually to read new entries.' : ''}</p>
    </>}
    <DialogFooter><Button variant="outline" onClick={() => setPaused(value => !value)}>{paused ? <Play aria-hidden="true" /> : <Pause aria-hidden="true" />}{paused ? 'Resume auto-refresh' : 'Pause auto-refresh'}</Button><Button variant="outline" disabled={busy} onClick={() => setRefresh(value => value + 1)}><RefreshCw aria-hidden="true" />Refresh logs</Button><Button variant="outline" onClick={onClose}>Close logs</Button></DialogFooter>
  </DialogContent></Dialog>
}
