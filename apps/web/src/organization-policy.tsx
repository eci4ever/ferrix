import { useEffect, useState } from 'react'
import { request } from '@/lib/api'
import { Card, CardHeader, CardTitle, CardDescription, CardContent } from '@/components/ui/card'
import { Field, FieldLabel } from '@/components/ui/field'
import { Input } from '@/components/ui/input'
import { Button } from '@/components/ui/button'
import { Alert, AlertTitle } from '@/components/ui/alert'
export function OrganizationPolicy({ path, canEdit, refreshVersion }: { path: string; canEdit: boolean; refreshVersion: number }) {
  const [limits, setLimits] = useState<{ member_limit: number | null; invitation_limit: number | null } | null>(null), [message, setMessage] = useState(''), [busy, setBusy] = useState(false), [version, setVersion] = useState(0)
  useEffect(() => { const abort = new AbortController(); request<typeof limits>(path, { signal: abort.signal }).then(value => { if (!abort.signal.aborted) setLimits(value) }).catch(reason => { if (!abort.signal.aborted) setMessage(reason.message) }); return () => abort.abort() }, [path, refreshVersion, version])
  return <Card><CardHeader><CardTitle>Organization limits</CardTitle><CardDescription>Blank means unlimited. Current memberships and pending invitations cannot exceed a new limit. Changes require recent identity verification.</CardDescription></CardHeader><CardContent>
    {message ? <Alert><AlertTitle>{message}</AlertTitle></Alert> : null}
    {!limits ? <Button variant="outline" onClick={() => setVersion(value => value + 1)}>Reload limits</Button> : <form key={JSON.stringify(limits)} className="flex max-w-xl flex-col gap-4" onSubmit={async event => {
      event.preventDefault(); const values = new FormData(event.currentTarget); const body = Object.fromEntries(['member_limit', 'invitation_limit'].map(key => [key, values.get(key) ? Number(values.get(key)) : null])); setBusy(true); setMessage('')
      try { await request(path, { method: 'POST', headers: { 'Content-Type': 'application/json' }, body: JSON.stringify(body) }); setMessage('Organization limits saved.'); setVersion(value => value + 1) } catch (reason) { setMessage(reason instanceof Error ? reason.message : 'Could not save limits.') } finally { setBusy(false) }
    }}>
      <Field><FieldLabel htmlFor={`${path}-members`}>Member limit</FieldLabel><Input id={`${path}-members`} name="member_limit" type="number" min={1} max={100000} defaultValue={limits.member_limit ?? ''} disabled={!canEdit || busy} /></Field>
      <Field><FieldLabel htmlFor={`${path}-invitations`}>Pending invitation limit</FieldLabel><Input id={`${path}-invitations`} name="invitation_limit" type="number" min={1} max={100000} defaultValue={limits.invitation_limit ?? ''} disabled={!canEdit || busy} /></Field>
      {canEdit ? <Button disabled={busy} className="self-start">{busy ? 'Saving…' : 'Save organization limits'}</Button> : null}
    </form>}
  </CardContent></Card>
}
