import { useState } from 'react'
import { request } from '@/lib/api'
import type { ScalableService } from '@/swarm-scale-dialog'
import { Dialog, DialogContent, DialogDescription, DialogFooter, DialogHeader, DialogTitle } from '@/components/ui/dialog'
import { Alert, AlertDescription, AlertTitle } from '@/components/ui/alert'
import { Button } from '@/components/ui/button'

export function SwarmRestartDialog({ service, onClose, onRestarted }: { service: ScalableService; onClose: () => void; onRestarted: () => void }) {
  const [busy, setBusy] = useState(false)
  const [error, setError] = useState('')
  async function restart() {
    if (busy) return
    setBusy(true); setError('')
    try {
      await request(`/docker/swarm/services/${encodeURIComponent(service.id)}/restart`, { method: 'POST', headers: { 'Content-Type': 'application/json' }, body: JSON.stringify({ version: service.version }) })
      onRestarted()
    } catch (reason) { setError(reason instanceof Error ? reason.message : 'Could not restart service.'); setBusy(false) }
  }
  return <Dialog open onOpenChange={open => { if (!open && !busy) onClose() }}><DialogContent showCloseButton={!busy} onEscapeKeyDown={event => { if (busy) event.preventDefault() }} onInteractOutside={event => { if (busy) event.preventDefault() }}>
    <DialogHeader><DialogTitle>Restart {service.name || service.id}?</DialogTitle><DialogDescription>Docker will replace this service’s tasks using its existing update policy. The replica count and configuration stay the same. Connections may be interrupted during the restart.</DialogDescription></DialogHeader>
    {error && <Alert variant="destructive"><AlertTitle>Service could not be restarted</AlertTitle><AlertDescription>{error}</AlertDescription></Alert>}
    <DialogFooter><Button type="button" variant="outline" disabled={busy} onClick={onClose}>Cancel</Button><Button disabled={busy} onClick={restart}>{busy ? 'Restarting…' : 'Restart service'}</Button></DialogFooter>
  </DialogContent></Dialog>
}
