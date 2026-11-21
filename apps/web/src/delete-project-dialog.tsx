import { useState } from 'react'
import type { FormEvent } from 'react'
import type { Project } from '@/lib/api'
import { request } from '@/lib/api'
import { Dialog, DialogContent, DialogHeader, DialogTitle, DialogDescription, DialogFooter } from '@/components/ui/dialog'
import { Button } from '@/components/ui/button'
import { Input } from '@/components/ui/input'
import { Field, FieldLabel, FieldDescription } from '@/components/ui/field'
import { Alert, AlertTitle, AlertDescription } from '@/components/ui/alert'

export function DeleteProjectDialog({ project, onClose, onDeleted }: { project: Project; onClose: () => void; onDeleted: () => void }) {
  const [name, setName] = useState('')
  const [busy, setBusy] = useState(false)
  const [error, setError] = useState('')
  const blocked = project.service_count > 0
  async function remove(event: FormEvent) {
    event.preventDefault()
    if (busy || blocked || name !== project.name) return
    setBusy(true); setError('')
    try { await request(`/projects/${project.id}`, { method: 'DELETE', headers: { 'Content-Type': 'application/json' }, body: JSON.stringify({ name }) }); onDeleted() }
    catch (reason) { setError(reason instanceof Error ? reason.message : 'Could not delete project.'); setBusy(false) }
  }
  return <Dialog open onOpenChange={open => { if (!open && !busy) onClose() }}><DialogContent showCloseButton={!busy} onEscapeKeyDown={e => { if (busy) e.preventDefault() }} onInteractOutside={e => { if (busy) e.preventDefault() }}><DialogHeader><DialogTitle>Delete {project.name}?</DialogTitle><DialogDescription>This permanently removes the project and its environments. This action cannot be undone.</DialogDescription></DialogHeader>
    {blocked && <Alert><AlertTitle>Delete services first</AlertTitle><AlertDescription>This project has {project.service_count} {project.service_count === 1 ? 'service' : 'services'}. Remove all services from every environment before deleting this project.</AlertDescription></Alert>}
    {error && <Alert variant="destructive"><AlertTitle>Project could not be deleted</AlertTitle><AlertDescription>{error}</AlertDescription></Alert>}
    <form onSubmit={remove} className="grid gap-5"><Field><FieldLabel htmlFor="delete-project-name">Type the project name to confirm</FieldLabel><Input id="delete-project-name" value={name} onChange={e => setName(e.target.value)} disabled={busy || blocked} autoComplete="off" /><FieldDescription>Enter {project.name} exactly.</FieldDescription></Field><DialogFooter><Button type="button" variant="outline" disabled={busy} onClick={onClose}>Cancel</Button><Button type="submit" variant="destructive" disabled={busy || blocked || name !== project.name}>{busy ? 'Deleting…' : 'Delete project'}</Button></DialogFooter></form>
  </DialogContent></Dialog>
}
