import { useState } from 'react'
import type { FormEvent } from 'react'
import { request } from '@/lib/api'
import { Dialog, DialogContent, DialogDescription, DialogFooter, DialogHeader, DialogTitle } from '@/components/ui/dialog'
import { Alert, AlertDescription, AlertTitle } from '@/components/ui/alert'
import { Button } from '@/components/ui/button'
import { Input } from '@/components/ui/input'
import { Field, FieldDescription, FieldLabel } from '@/components/ui/field'

export type ScalableService = { id: string; name: string | null; replicas: number | null; version: number | null }

export function SwarmScaleDialog({ service, onClose, onScaled }: { service: ScalableService; onClose: () => void; onScaled: (replicas: number) => void }) {
  const [value, setValue] = useState(String(service.replicas ?? ''))
  const [busy, setBusy] = useState(false)
  const [error, setError] = useState('')
  const replicas = Number(value)
  const valid = value.trim() !== '' && Number.isInteger(replicas) && replicas >= 0 && replicas <= 100 && replicas !== service.replicas
  async function submit(event: FormEvent) {
    event.preventDefault()
    if (!valid || busy) return
    setBusy(true); setError('')
    try {
      await request(`/docker/swarm/services/${encodeURIComponent(service.id)}/scale`, { method: 'POST', headers: { 'Content-Type': 'application/json' }, body: JSON.stringify({ replicas, version: service.version }) })
      onScaled(replicas)
    } catch (reason) { setError(reason instanceof Error ? reason.message : 'Could not scale service.'); setBusy(false) }
  }
  return <Dialog open onOpenChange={open => { if (!open && !busy) onClose() }}><DialogContent showCloseButton={!busy} onEscapeKeyDown={event => { if (busy) event.preventDefault() }} onInteractOutside={event => { if (busy) event.preventDefault() }}>
    <DialogHeader><DialogTitle>Scale {service.name || service.id}</DialogTitle><DialogDescription>Change the desired replica count for this Swarm service. Docker schedules or removes tasks to match the new count.</DialogDescription></DialogHeader>
    {error && <Alert variant="destructive"><AlertTitle>Service could not be scaled</AlertTitle><AlertDescription>{error}</AlertDescription></Alert>}
    <form className="grid gap-5" onSubmit={submit}>
      <Field><FieldLabel htmlFor="swarm-replicas">Replicas</FieldLabel><Input id="swarm-replicas" type="number" min={0} max={100} step={1} required value={value} onChange={event => setValue(event.target.value)} disabled={busy} /><FieldDescription>Current desired replicas: {service.replicas ?? 'unknown'}. Enter a whole number from 0 to 100.</FieldDescription></Field>
      {replicas === 0 && value !== '' && <Alert variant="destructive"><AlertTitle>This stops all service tasks</AlertTitle><AlertDescription>The service will be unavailable until replicas are increased again. Persistent storage is retained by Docker.</AlertDescription></Alert>}
      <p className="text-sm">Confirm changing {service.name || service.id} from {service.replicas ?? 'unknown'} to {valid ? replicas : '…'} replicas.</p>
      <DialogFooter><Button type="button" variant="outline" disabled={busy} onClick={onClose}>Cancel</Button><Button type="submit" disabled={!valid || busy}>{busy ? 'Scaling…' : 'Scale service'}</Button></DialogFooter>
    </form>
  </DialogContent></Dialog>
}
