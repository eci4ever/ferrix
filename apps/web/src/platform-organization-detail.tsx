import { useEffect, useState } from 'react'
import type { FormEvent } from 'react'
import { Link } from '@tanstack/react-router'
import type { Identity } from './auth'
import { ActivityPage } from './activity'
import { request } from '@/lib/api'
import { FormDialog } from '@/components/form-dialog'
import { PagedTable } from '@/components/paged-table'
import { OrganizationPolicy } from './organization-policy'
import { Button } from '@/components/ui/button'
import { Input } from '@/components/ui/input'
import { Field, FieldLabel } from '@/components/ui/field'
import { NativeSelect, NativeSelectOption } from '@/components/ui/native-select'
import { Alert, AlertDescription, AlertTitle } from '@/components/ui/alert'
import { Card, CardHeader, CardTitle, CardDescription, CardContent } from '@/components/ui/card'
import { Badge } from '@/components/ui/badge'
import { DialogFooter } from '@/components/ui/dialog'
import { Tabs, TabsList, TabsTrigger, TabsContent } from '@/components/ui/tabs'

type Member = { id: string; name: string; email: string; role: string; banned: boolean }
type Invitation = { id: string; email: string; role: string; expires_at: string; status?: string; inviter_name?: string; delivery_status?: string; delivery_attempted_at?: string }
type Detail = { organization: { id: string; name: string; logo_url: string | null; created_at: string; is_default: boolean; project_count: number; member_count?: number }; members?: Member[]; invitations?: Invitation[] }
type Operation = { kind: 'role' | 'remove'; member: Member } | { kind: 'renew' | 'revoke'; invitation: Invitation } | { kind: 'invite' }
export function PlatformOrganizationDetail({ organizationId, identity, refreshVersion, refresh }: { organizationId: string; identity: Identity; refreshVersion: number; refresh: () => Promise<void> }) {
  const [detail, setDetail] = useState<Detail | null>(null), [error, setError] = useState(''), [message, setMessage] = useState(''), [version, setVersion] = useState(0)
  const [tab, setTab] = useState('overview'), [operation, setOperation] = useState<Operation | null>(null), [dialogError, setDialogError] = useState(''), [busy, setBusy] = useState(false)
  const [token, setToken] = useState(''), [delivery, setDelivery] = useState('')
  const admin = identity.system_role === 'admin', base = `/auth/admin/organizations/${organizationId}`
  useEffect(() => {
    if (!admin) return
    const abort = new AbortController(); setError('')
    request<Detail>(base, { signal: abort.signal }).then(value => { if (!abort.signal.aborted) setDetail(value) }).catch(reason => { if (!abort.signal.aborted) { setDetail(null); setError(reason.message) } })
    return () => abort.abort()
  }, [admin, base, refreshVersion, version])
  function open(value: Operation) { setDialogError(''); setOperation(value) }
  async function save(path: string, body: object, invitation = false) {
    setBusy(true); setDialogError(''); setMessage('')
    try {
      const result = await request<{ token?: string; email_delivery?: string } | undefined>(base + path, { method: 'POST', headers: { 'Content-Type': 'application/json' }, body: JSON.stringify(body), signal: AbortSignal.timeout(invitation ? 30_000 : 10_000) })
      if (result?.token) { setToken(result.token); setDelivery(result.email_delivery === 'sent' ? 'Email accepted by the provider.' : result.email_delivery === 'failed' ? 'Email delivery could not be confirmed. Share this valid token privately.' : 'Email is not configured. Share this token privately.') }
      setOperation(null); setMessage('Organization updated.'); setVersion(value => value + 1)
      try { await refresh() } catch { setMessage('Saved. Refresh workspace to reload the organization list.') }
    } catch (reason) { const text = reason instanceof Error ? reason.message : 'Could not update organization.'; if (operation) setDialogError(text); else setError(text) }
    finally { setBusy(false) }
  }
  function submit(event: FormEvent<HTMLFormElement>) {
    event.preventDefault(); if (!operation) return
    const values = Object.fromEntries(new FormData(event.currentTarget))
    if (operation.kind === 'invite') return save('/invitations', values, true)
    if ('member' in operation) return save(`/members/${operation.member.id}/${operation.kind}`, operation.kind === 'role' ? { role: values.role } : {})
    return save(`/invitations/${operation.invitation.id}/${operation.kind}`, {}, operation.kind === 'renew')
  }
  if (!admin) return <Alert><AlertTitle>Platform administrator access required</AlertTitle><AlertDescription>Only platform administrators can manage this organization.</AlertDescription></Alert>
  return <div className="flex min-w-0 flex-col gap-6">
    <Link to="/platform/organizations" className="text-sm underline underline-offset-4">Back to organizations</Link>
    <div className="space-y-1"><h1 className="break-words text-2xl font-semibold">{detail?.organization.name ?? 'Organization detail'}</h1><p className="text-sm text-muted-foreground">Platform administration. Your active organization and memberships stay unchanged unless you explicitly manage your own membership.</p></div>
    {error ? <Alert variant="destructive"><AlertTitle>Organization needs attention</AlertTitle><AlertDescription>{error}<Button variant="outline" onClick={() => setVersion(value => value + 1)}>Retry organization</Button></AlertDescription></Alert> : null}
    {message ? <Alert role="status"><AlertTitle>{message}</AlertTitle></Alert> : null}
    {token ? <Alert><AlertTitle>Invitation token</AlertTitle><AlertDescription><p>{delivery}</p><code className="break-all select-all">{token}</code><Button variant="outline" onClick={() => setToken('')}>Dismiss token</Button></AlertDescription></Alert> : null}
    {!detail ? !error ? <p role="status">Loading organization…</p> : null : <Tabs value={tab} onValueChange={setTab} className="min-w-0 gap-6">
      <div className="overflow-x-auto pb-1"><TabsList aria-label="Platform organization sections">{['overview', 'members', 'invitations', 'activity'].map(value => <TabsTrigger key={value} value={value}>{value[0].toUpperCase() + value.slice(1)}</TabsTrigger>)}</TabsList></div>
      <TabsContent value="overview" className="space-y-6"><Card><CardHeader><CardTitle>Organization overview</CardTitle><CardDescription>{detail.organization.member_count ?? detail.members?.length ?? 0} members · {detail.organization.project_count} projects{detail.organization.is_default ? ' · Default organization' : ''}</CardDescription></CardHeader><CardContent>
        <form key={`${detail.organization.name}:${detail.organization.logo_url}`} className="flex max-w-xl flex-col gap-4" onSubmit={event => { event.preventDefault(); const values = Object.fromEntries(new FormData(event.currentTarget)); void save('', { name: values.name, logo_url: values.logo_url || null }) }}>
          <Field><FieldLabel htmlFor="detail-org-name">Organization name</FieldLabel><Input id="detail-org-name" name="name" maxLength={80} defaultValue={detail.organization.name} required disabled={busy} /></Field>
          <Field><FieldLabel htmlFor="detail-org-logo">Logo URL</FieldLabel><Input id="detail-org-logo" name="logo_url" type="url" maxLength={2048} defaultValue={detail.organization.logo_url ?? ''} disabled={busy} /></Field>
          <p className="text-sm text-muted-foreground">Created {new Date(detail.organization.created_at).toLocaleString()}. Changes require recent identity verification in Security.</p>
          <Button className="self-start" disabled={busy}>{busy ? 'Saving…' : 'Save organization'}</Button>
        </form>
      </CardContent></Card><OrganizationPolicy path={`${base}/policy`} canEdit={true} refreshVersion={refreshVersion + version} /></TabsContent>
      <TabsContent value="members"><PagedTable<Member> label="Organization members" path={`${base}/members`} roles refreshVersion={refreshVersion + version} columns={[
        { id: 'name', label: 'Member', value: member => `${member.name} ${member.email}`, cell: member => <div><p>{member.name}</p><p className="text-xs text-muted-foreground">{member.email}</p></div> },
        { id: 'role', label: 'Role', value: member => member.role }, { id: 'status', label: 'Status', cell: member => <Badge variant="outline">{member.banned ? 'Banned' : 'Active'}</Badge> },
        { id: 'actions', label: 'Actions', cell: member => <div className="flex flex-wrap gap-2"><Button variant="outline" disabled={busy || member.banned} onClick={() => open({ kind: 'role', member })} aria-label={`Change role for ${member.email}`}>Change role</Button><Button variant="outline" disabled={busy || (member.id === identity.user_id && organizationId === identity.organization_id)} onClick={() => open({ kind: 'remove', member })} aria-label={`Remove ${member.email}`}>Remove</Button></div> },
      ]} /></TabsContent>
      <TabsContent value="invitations" className="space-y-4"><p className="text-sm text-muted-foreground">Invitation history includes accepted, expired, cancelled and rejected invitations. Pending tokens expire after two days. Sending and regenerating attempt email delivery using Platform Settings.</p><Button disabled={busy} onClick={() => open({ kind: 'invite' })}>Invite member</Button><PagedTable<Invitation> label="Organization invitations" path={`${base}/invitations`} roles statuses refreshVersion={refreshVersion + version} columns={[
        { id: 'email', label: 'Email', value: invite => invite.email }, { id: 'status', label: 'Status', cell: invite => invite.status ?? 'pending' }, { id: 'inviter', label: 'Invited by', cell: invite => invite.inviter_name ?? 'Unknown' }, { id: 'delivery', label: 'Email delivery', cell: invite => invite.delivery_status ?? 'unknown' }, { id: 'role', label: 'Role', value: invite => invite.role }, { id: 'expires', label: 'Expires', cell: invite => <time dateTime={invite.expires_at}>{new Date(invite.expires_at).toLocaleString()}</time> },
        { id: 'actions', label: 'Actions', cell: invitation => <div className="flex flex-wrap gap-2"><Button variant="outline" disabled={busy || (!!invitation.status && !['pending','expired'].includes(invitation.status))} onClick={() => open({ kind: 'renew', invitation })}>Regenerate</Button><Button variant="outline" disabled={busy || (!!invitation.status && invitation.status !== 'pending')} onClick={() => open({ kind: 'revoke', invitation })}>Cancel invitation</Button></div> },
      ]} /></TabsContent>
      <TabsContent value="activity"><ActivityPage key={organizationId} identity={identity} organizationId={organizationId} refreshVersion={refreshVersion + version} /></TabsContent>
    </Tabs>}
    <FormDialog open={!!operation} onOpenChange={value => { if (!value) setOperation(null) }} title={operation?.kind === 'invite' ? 'Invite member' : operation?.kind === 'role' ? 'Change member role' : operation?.kind === 'remove' ? 'Remove member?' : operation?.kind === 'renew' ? 'Regenerate invitation?' : 'Cancel invitation?'} description={operation?.kind === 'remove' ? 'Removes only this membership and sessions using this organization. The last owner and platform administrator access are protected.' : operation?.kind === 'renew' ? 'Invalidates the old token and attempts to email a replacement valid for two days.' : 'Changes apply only to this organization and require recent identity verification.'} busy={busy}>
      {dialogError ? <Alert variant="destructive"><AlertTitle>Action could not be completed</AlertTitle><AlertDescription>{dialogError}</AlertDescription></Alert> : null}
      {operation ? <form className="space-y-4" onSubmit={event => void submit(event)}>
        {operation.kind === 'invite' ? <Field><FieldLabel htmlFor="detail-invite-email">Invitation email</FieldLabel><Input id="detail-invite-email" name="email" type="email" maxLength={254} required disabled={busy} /></Field> : null}
        {operation.kind === 'invite' || operation.kind === 'role' ? <Field><FieldLabel htmlFor="detail-member-role">Role</FieldLabel><NativeSelect id="detail-member-role" name="role" defaultValue={operation.kind === 'role' ? operation.member.role : 'member'} disabled={busy}>{(operation.kind === 'role' ? ['owner', 'admin', 'member'] : ['admin', 'member']).map(role => <NativeSelectOption key={role} value={role}>{role}</NativeSelectOption>)}</NativeSelect></Field> : null}
        <DialogFooter><Button type="button" variant="outline" disabled={busy} onClick={() => setOperation(null)}>Cancel</Button><Button disabled={busy} variant={operation.kind === 'remove' || operation.kind === 'revoke' ? 'destructive' : 'default'}>{busy ? 'Saving…' : 'Confirm'}</Button></DialogFooter>
      </form> : null}
    </FormDialog>
  </div>
}
