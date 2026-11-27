import { useState } from 'react'
import type { FormEvent } from 'react'
import type { Project } from '@/lib/api'
import { request } from '@/lib/api'
import { AlertDialog, AlertDialogContent, AlertDialogHeader, AlertDialogTitle, AlertDialogDescription, AlertDialogFooter, AlertDialogCancel } from '@/components/ui/alert-dialog'
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
  return <AlertDialog open onOpenChange={open => { if (!open && !busy) onClose() }}><AlertDialogContent className="max-h-[90dvh] overflow-y-auto" onEscapeKeyDown={e => { if (busy) e.preventDefault() }}><AlertDialogHeader><AlertDialogTitle>Delete {project.name}?</AlertDialogTitle><AlertDialogDescription>This permanently removes the project and its environments. This action cannot be undone.</AlertDialogDescription></AlertDialogHeader>
    {blocked && <Alert><AlertTitle>Delete services first</AlertTitle><AlertDescription>This project has {project.service_count} {project.service_count === 1 ? 'service' : 'services'}. Remove all services from every environment before deleting this project.</AlertDescription></Alert>}
    {error && <Alert variant="destructive"><AlertTitle>Project could not be deleted</AlertTitle><AlertDescription>{error}</AlertDescription></Alert>}
    <form onSubmit={remove} className="grid gap-5"><Field><FieldLabel htmlFor="delete-project-name">Type the project name to confirm</FieldLabel><Input id="delete-project-name" value={name} onChange={e => setName(e.target.value)} disabled={busy || blocked} autoComplete="off" /><FieldDescription>Enter {project.name} exactly.</FieldDescription></Field><AlertDialogFooter><AlertDialogCancel type="button" disabled={busy}>Cancel</AlertDialogCancel><Button type="submit" variant="destructive" disabled={busy || blocked || name !== project.name}>{busy ? 'Deleting…' : 'Delete project'}</Button></AlertDialogFooter></form>
  </AlertDialogContent></AlertDialog>
}
