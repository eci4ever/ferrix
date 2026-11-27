import { useEffect, useState } from 'react'
import type { FormEvent } from 'react'
import { request } from '@/lib/api'
import { DataTable } from '@/components/data-table'
import { FormDialog } from '@/components/form-dialog'
import { Alert, AlertTitle, AlertDescription } from '@/components/ui/alert'
import { Button } from '@/components/ui/button'
import { Badge } from '@/components/ui/badge'
import { Field, FieldLabel } from '@/components/ui/field'
import { Input } from '@/components/ui/input'
import { DialogFooter } from '@/components/ui/dialog'

type Organization = { id: string; name: string; logo_url: string | null; is_default: boolean; member_count: number; project_count: number }
type Operation = { kind: 'create' } | { kind: 'edit' | 'delete'; org: Organization }
export function PlatformOrganizations({ admin, activeOrganization, refreshVersion, refresh }: { admin: boolean; activeOrganization: string; refreshVersion: number; refresh: () => Promise<void> }) {
  const [items, setItems] = useState<Organization[]>([]), [loading, setLoading] = useState(true), [error, setError] = useState(''), [dialogError, setDialogError] = useState(''), [message, setMessage] = useState('')
  const [operation, setOperation] = useState<Operation | null>(null), [busy, setBusy] = useState(false), [retry, setRetry] = useState(0)
  useEffect(() => {
    if (!admin) return
    const abort = new AbortController()
    setLoading(true); setError('')
    request<Organization[]>('/auth/admin/organizations', { signal: abort.signal }).then(value => { if (!abort.signal.aborted) { setItems(value); setLoading(false) } }).catch(reason => { if (!abort.signal.aborted) { setError(reason.message); setLoading(false) } })
    return () => abort.abort()
  }, [admin, refreshVersion, retry])
  function open(value: Operation) { setDialogError(''); setOperation(value) }
  async function submit(event: FormEvent<HTMLFormElement>) {
    event.preventDefault()
    if (!operation) return
    const input = Object.fromEntries(new FormData(event.currentTarget))
    const path = operation.kind === 'create' ? '' : `/${operation.org.id}${operation.kind === 'delete' ? '/delete' : ''}`
    setBusy(true); setDialogError(''); setMessage('')
    try {
      await request(`/auth/admin/organizations${path}`, { method: 'POST', headers: { 'Content-Type': 'application/json' }, body: JSON.stringify(operation.kind === 'edit' ? { name: input.name, logo_url: input.logo_url || null } : input) })
      setOperation(null); setMessage('Organization updated.'); await refresh()
    } catch (reason) { setDialogError(reason instanceof Error ? reason.message : 'Could not update organization.') }
    finally { setBusy(false) }
  }
  if (!admin) return <Alert><AlertTitle>Platform administrator access required</AlertTitle><AlertDescription>Only platform administrators can manage all organizations.</AlertDescription></Alert>
  return <div className="flex min-w-0 flex-col gap-6">
    <div className="flex flex-wrap items-start justify-between gap-3"><div className="space-y-1"><h1 className="text-2xl font-semibold">Organizations</h1><p className="text-sm text-muted-foreground">Manage every organization on this Ferrix installation.</p></div><Button disabled={busy} onClick={() => open({ kind: 'create' })}>Create organization</Button></div>
    {message ? <Alert role="status"><AlertTitle>{message}</AlertTitle></Alert> : null}
    {error ? <Alert variant="destructive"><AlertTitle>Organizations unavailable</AlertTitle><AlertDescription>{error}<Button variant="outline" onClick={() => setRetry(value => value + 1)}>Retry organizations</Button></AlertDescription></Alert> : null}
    <DataTable label="Platform organizations" data={items} loading={loading} columns={[
      { id: 'name', label: 'Organization', value: org => org.name, cell: org => <div className="flex flex-wrap items-center gap-2"><span className="break-words">{org.name}</span>{org.is_default ? <Badge variant="secondary">Default</Badge> : null}{org.id === activeOrganization ? <Badge variant="outline">Current</Badge> : null}</div> },
      { id: 'members', label: 'Members', cell: org => org.member_count },
      { id: 'projects', label: 'Projects', cell: org => org.project_count },
      { id: 'actions', label: 'Actions', cell: org => <div className="flex flex-wrap gap-2"><Button variant="outline" disabled={busy} aria-label={`Edit ${org.name}`} onClick={() => open({ kind: 'edit', org })}>Edit</Button><Button variant="outline" disabled={busy || org.is_default || org.project_count > 0 || org.id === activeOrganization} title={org.is_default ? 'Default organization cannot be deleted' : org.project_count > 0 ? 'Delete its projects first' : org.id === activeOrganization ? 'Switch to another organization first' : undefined} aria-label={`Delete ${org.name}`} onClick={() => open({ kind: 'delete', org })}>Delete</Button></div> },
    ]} />
    <p className="text-sm text-muted-foreground">Deleting an organization retains user accounts and signs out sessions using that organization. Default organizations and organizations with projects are protected.</p>
    <FormDialog open={!!operation} onOpenChange={open => { if (!open) setOperation(null) }} title={operation?.kind === 'create' ? 'Create organization' : operation?.kind === 'edit' ? 'Edit organization' : 'Delete organization?'} description={operation?.kind === 'create' ? 'Creates an organization with you as its owner. Your current organization stays selected.' : operation?.kind === 'edit' ? 'Update the name and logo without changing organization memberships.' : 'Permanently deletes memberships and invitations. User accounts are retained. Enter the exact organization name to confirm.'} busy={busy}>
      {dialogError ? <Alert variant="destructive"><AlertTitle>Organization could not be updated</AlertTitle><AlertDescription>{dialogError}</AlertDescription></Alert> : null}
      {operation ? <form key={operation.kind === 'create' ? 'create' : `${operation.kind}:${operation.org.id}`} className="flex flex-col gap-4" onSubmit={event => void submit(event)}><fieldset disabled={busy} className="flex flex-col gap-4">
        {operation.kind === 'delete' ? <Field><FieldLabel htmlFor="platform-org-confirmation">Type {operation.org.name} to confirm</FieldLabel><Input id="platform-org-confirmation" name="confirmation" required autoFocus /></Field> : <><Field><FieldLabel htmlFor="platform-org-name">Organization name</FieldLabel><Input id="platform-org-name" name="name" defaultValue={operation.kind === 'edit' ? operation.org.name : ''} maxLength={80} required autoFocus /></Field>{operation.kind === 'edit' ? <Field><FieldLabel htmlFor="platform-org-logo">Logo URL</FieldLabel><Input id="platform-org-logo" name="logo_url" type="url" maxLength={2048} defaultValue={operation.org.logo_url ?? ''} /></Field> : null}</>}
      </fieldset><DialogFooter><Button type="button" variant="outline" disabled={busy} onClick={() => setOperation(null)}>Cancel</Button><Button type="submit" disabled={busy} variant={operation.kind === 'delete' ? 'destructive' : 'default'}>{busy ? 'Saving…' : operation.kind === 'delete' ? 'Delete organization' : operation.kind === 'create' ? 'Create organization' : 'Save organization'}</Button></DialogFooter></form> : null}
    </FormDialog>
  </div>
}
