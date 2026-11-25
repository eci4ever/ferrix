import { useEffect, useState } from 'react'
import type { FormEvent } from 'react'
import { request } from '@/lib/api'
import { Alert, AlertDescription, AlertTitle } from '@/components/ui/alert'
import { Button } from '@/components/ui/button'
import { Card, CardContent, CardDescription, CardHeader, CardTitle } from '@/components/ui/card'
import { Field, FieldDescription, FieldGroup, FieldLabel } from '@/components/ui/field'
import { Textarea } from '@/components/ui/textarea'

type Notes = { notes: string; revision: number }
export function ServiceNotes({ serviceId, canManage }: { serviceId: string; canManage: boolean }) {
  const [saved, setSaved] = useState<Notes | null>(null)
  const [draft, setDraft] = useState('')
  const [error, setError] = useState('')
  const [notice, setNotice] = useState('')
  const [busy, setBusy] = useState(false)
  const [reload, setReload] = useState(0)
  useEffect(() => {
    const controller = new AbortController()
    setBusy(true); setError('')
    request<Notes>(`/services/${serviceId}/notes`, { signal: controller.signal }).then(value => {
      if (!controller.signal.aborted) { setSaved(value); setDraft(value.notes); setNotice('') }
    }).catch(e => { if (!controller.signal.aborted) setError(e instanceof Error ? e.message : 'Could not load notes.') }).finally(() => { if (!controller.signal.aborted) setBusy(false) })
    return () => controller.abort()
  }, [serviceId, reload])
  const bytes = new TextEncoder().encode(draft).length
  const dirty = saved !== null && draft !== saved.notes
  async function save(event: FormEvent) {
    event.preventDefault()
    if (!saved || busy || bytes > 32768) return
    setBusy(true); setError(''); setNotice('')
    try {
      const value = await request<Notes>(`/services/${serviceId}/notes`, { method: 'POST', headers: { 'Content-Type': 'application/json' }, body: JSON.stringify({ notes: draft, revision: saved.revision }) })
      setSaved(value); setDraft(value.notes); setNotice('Notes saved. No deployment needed.')
    } catch (e) { setError(e instanceof Error ? e.message : 'Could not save notes.') }
    finally { setBusy(false) }
  }
  return <Card><CardHeader><CardTitle><h2>Service notes</h2></CardTitle><CardDescription>Keep setup instructions, useful links and reminders for your team. Notes are shared with organization members. Do not store passwords or API keys here.</CardDescription></CardHeader><CardContent className="flex flex-col gap-4">
    {error ? <Alert variant="destructive"><AlertTitle>Unable to continue</AlertTitle><AlertDescription><p>{error}</p><Button type="button" variant="outline" disabled={busy} onClick={() => setReload(value => value + 1)}>{dirty ? 'Discard draft and load saved notes' : 'Retry loading notes'}</Button></AlertDescription></Alert> : null}
    {saved ? <form onSubmit={save} className="flex flex-col gap-4"><FieldGroup><Field data-invalid={bytes > 32768 || undefined}><FieldLabel htmlFor="service-notes">Notes</FieldLabel><Textarea id="service-notes" value={draft} readOnly={!canManage} disabled={busy} onChange={event => { setDraft(event.target.value); setNotice('') }} aria-invalid={bytes > 32768} aria-describedby="service-notes-help" rows={12} maxLength={32768} placeholder={canManage ? 'Add notes about this service…' : 'No notes yet.'} /><FieldDescription id="service-notes-help">Plain text · {bytes.toLocaleString()} / 32,768 bytes{!canManage ? ' · Only organization owners and admins can edit.' : dirty ? ' · Unsaved changes' : ''}</FieldDescription></Field></FieldGroup>{bytes > 32768 ? <p role="alert" className="text-sm text-destructive">Notes exceed the 32 KiB limit.</p> : null}{canManage ? <Button className="self-end" disabled={busy || !dirty || bytes > 32768}>{busy ? 'Saving…' : 'Save notes'}</Button> : null}</form> : !error ? <p role="status" className="text-sm text-muted-foreground">Loading notes…</p> : null}
    <p role="status" className="text-sm text-muted-foreground">{notice}</p>
  </CardContent></Card>
}
