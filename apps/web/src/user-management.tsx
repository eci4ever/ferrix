import { useEffect, useId, useState } from 'react'
import type { FormEvent, ReactNode } from 'react'
import type { Identity, Organization, Member, Invitation, Session } from './auth'
import { FormDialog } from '@/components/form-dialog'
import { request } from '@/lib/api'
import { DataTable } from '@/components/data-table'
import { Button } from '@/components/ui/button'
import { Input } from '@/components/ui/input'
import { Field, FieldGroup, FieldLabel, FieldDescription } from '@/components/ui/field'
import { Badge } from '@/components/ui/badge'
import { Alert, AlertDescription, AlertTitle } from '@/components/ui/alert'
import { Card, CardHeader, CardTitle, CardDescription, CardContent } from '@/components/ui/card'
import { Dialog, DialogContent, DialogHeader, DialogTitle, DialogDescription, DialogFooter } from '@/components/ui/dialog'
import { DropdownMenu, DropdownMenuTrigger, DropdownMenuContent, DropdownMenuGroup, DropdownMenuItem } from '@/components/ui/dropdown-menu'
import { NativeSelect, NativeSelectOption } from '@/components/ui/native-select'

type Refresh = () => Promise<unknown>
type User = { id: string; name: string; email: string; system_role: string; banned: boolean; ban_reason: string | null; ban_expires_at: string | null; created_at: string }
type UserDetail = { user: User; organizations: Organization[]; sessions: Session[] }
type OrgProfile = { id: string; name: string; logo_url: string | null; is_default: boolean }
function values(event: FormEvent<HTMLFormElement>) { event.preventDefault(); return Object.fromEntries(new FormData(event.currentTarget)) as Record<string, string> }
function Notice({ message }: { message: string }) { return message ? <Alert><AlertTitle>Account notice</AlertTitle><AlertDescription>{message}</AlertDescription></Alert> : null }
function Section({ title, description, children }: { title: string; description?: string; children: ReactNode }) { return <Card><CardHeader><CardTitle><h2>{title}</h2></CardTitle>{description ? <CardDescription>{description}</CardDescription> : null}</CardHeader><CardContent className="flex flex-col gap-4">{children}</CardContent></Card> }
function Text({ name, label, type = 'text', required = true, maxLength = 254, defaultValue = '' }: { name: string; label: string; type?: string; required?: boolean; maxLength?: number; defaultValue?: string }) { const id = useId(); return <Field><FieldLabel htmlFor={id}>{label}</FieldLabel><Input id={id} name={name} type={type} required={required} maxLength={maxLength} defaultValue={defaultValue} /></Field> }
function Choice({ label, value, onChange, options, disabled }: { label: string; value: string; onChange: (value: string) => void; options: [string, string][]; disabled?: boolean }) { const id = useId(); return <Field><FieldLabel htmlFor={id}>{label}</FieldLabel><NativeSelect id={id} value={value} onChange={event => onChange(event.target.value)} disabled={disabled}>{options.map(([value, label]) => <NativeSelectOption key={value} value={value}>{label}</NativeSelectOption>)}</NativeSelect></Field> }
function useActions(refresh: Refresh) {
  const [busy, setBusy] = useState(false); const [message, setMessage] = useState('')
  async function action(path: string, body = {}): Promise<{ token?: string } | null> {
    setBusy(true); setMessage('')
    try { const result = await request<{ token?: string } | undefined>(`/auth/${path}`, { method: 'POST', headers: { 'Content-Type': 'application/json' }, body: JSON.stringify(body) }); await refresh(); return result ?? {} }
    catch (error) { setMessage(error instanceof Error ? error.message : 'The action could not be completed.'); return null }
    finally { setBusy(false) }
  }
  return { busy, message, setMessage, action }
}
type Operation = { title: string; description: string; path: string; kind: 'ban' | 'delete' | 'confirm'; confirmation?: string; body?: object }
function OperationDialog({ operation, busy, message, close, submit }: { operation: Operation | null; busy: boolean; message: string; close: () => void; submit: (body: object) => Promise<void> }) {
  return <Dialog open={!!operation} onOpenChange={open => { if (!open && !busy) close() }}><DialogContent className="max-h-[calc(100svh-2rem)] overflow-y-auto" showCloseButton={!busy} onEscapeKeyDown={event => { if (busy) event.preventDefault() }} onInteractOutside={event => { if (busy) event.preventDefault() }}>
    <DialogHeader><DialogTitle>{operation?.title}</DialogTitle><DialogDescription>{operation?.description}</DialogDescription></DialogHeader><Notice message={message} />
    {operation ? <form key={operation.path} className="flex flex-col gap-4" onSubmit={async event => { const input = values(event); await submit(operation.kind === 'ban' ? { banned: true, reason: input.reason, expires_at: input.expires_at ? new Date(input.expires_at).toISOString() : null } : operation.kind === 'delete' ? { confirmation: input.confirmation } : operation.body ?? {}) }}>
      <fieldset disabled={busy}><FieldGroup>{operation.kind === 'ban' ? <><Text name="reason" label="Ban reason" maxLength={500} /><Text name="expires_at" label="Ban expiry (optional)" type="datetime-local" required={false} /><FieldDescription>Leave expiry empty for a permanent ban. Temporary bans expire automatically. Times use your browser’s timezone.</FieldDescription></> : null}{operation.kind === 'delete' ? <Text name="confirmation" label={`Type ${operation.confirmation} to confirm`} /> : null}</FieldGroup></fieldset>
      <div className="flex flex-wrap justify-end gap-2"><Button type="button" variant="outline" disabled={busy} onClick={close}>Cancel</Button><Button type="submit" variant={operation.kind === 'delete' ? 'destructive' : 'default'} disabled={busy}>{busy ? 'Please wait…' : 'Confirm'}</Button></div>
    </form> : null}
  </DialogContent></Dialog>
}

export function AdminPage({ identity, refresh }: { identity: Identity; refresh: Refresh }) {
  const [search, setSearch] = useState(''); const [role, setRole] = useState(''); const [status, setStatus] = useState(''); const [page, setPage] = useState(0); const [version, setVersion] = useState(0)
  const [result, setResult] = useState<{ items: User[]; total: number } | null>(null); const [loading, setLoading] = useState(true); const [loadError, setLoadError] = useState('')
  const [detail, setDetail] = useState<UserDetail | null>(null); const [detailId, setDetailId] = useState<string | null>(null); const [detailError, setDetailError] = useState('')
  const [operation, setOperation] = useState<Operation | null>(null); const [token, setToken] = useState('')
  const { busy, message, setMessage, action } = useActions(async () => { await refresh(); setVersion(value => value + 1) })
  useEffect(() => {
    if (identity.system_role !== 'admin') return
    const abort = new AbortController(); setLoading(true); setLoadError(''); setResult(null)
    const timer = setTimeout(() => { request<{ items: User[]; total: number }>(`/auth/admin/users?${new URLSearchParams({ q: search, role, status, page: String(page), limit: '10' })}`, { signal: abort.signal }).then(data => {
      if (abort.signal.aborted) return
      if (page > 0 && page * 10 >= data.total) { setPage(Math.max(0, Math.ceil(data.total / 10) - 1)); return }
      setResult(data); setLoading(false)
    }).catch(error => { if (!abort.signal.aborted) { setLoadError(error.message); setLoading(false) } }) }, 250)
    return () => { clearTimeout(timer); abort.abort() }
  }, [search, role, status, page, version, identity.system_role])
  useEffect(() => {
    setDetail(null); setDetailError(''); if (!detailId) return
    const abort = new AbortController()
    request<UserDetail>(`/auth/admin/users/${detailId}`, { signal: abort.signal }).then(value => { if (!abort.signal.aborted) setDetail(value) }).catch(error => { if (!abort.signal.aborted) setDetailError(error.message) })
    return () => abort.abort()
  }, [detailId, version])
  if (identity.system_role !== 'admin') return <Notice message="Platform administrator access is required." />
  const open = (value: Operation) => { setMessage(''); setOperation(value) }
  return <div className="flex flex-col gap-6"><h1 className="text-2xl font-semibold">Users</h1><Notice message={message} />
    {token ? <Alert><AlertTitle>Password reset token</AlertTitle><AlertDescription><p>Share privately with the account owner. Expires in 15 minutes; MFA remains required.</p><code className="break-all select-all">{token}</code><Button variant="outline" onClick={() => setToken('')}>Dismiss token</Button></AlertDescription></Alert> : null}
    <Section title="Platform users" description="Manage platform access separately from organization membership. Sensitive actions require recent identity verification from Security.">
      <div className="flex flex-wrap gap-4"><Choice label="Platform role" value={role} onChange={value => { setRole(value); setPage(0) }} options={[[ '', 'All roles' ], ['admin', 'Admin'], ['user', 'User']]} /><Choice label="Account status" value={status} onChange={value => { setStatus(value); setPage(0) }} options={[[ '', 'All statuses' ], ['active', 'Active'], ['banned', 'Banned']]} /></div>
      {loadError ? <><Notice message={loadError} /><Button variant="outline" onClick={() => setVersion(value => value + 1)}>Retry users</Button></> : null}
      <DataTable label="Users" loading={loading} data={result?.items ?? []} server={{ total: result?.total ?? 0, pageIndex: page, pageSize: 10, search, onSearchChange: value => { setSearch(value); setPage(0) }, onPageChange: setPage }} columns={[
        { id: 'user', label: 'User', value: user => `${user.name} ${user.email}`, cell: user => <div><p>{user.name}{user.id === identity.user_id ? ' (you)' : ''}</p><p className="text-xs text-muted-foreground">{user.email}</p></div> },
        { id: 'role', label: 'Platform role', value: user => user.system_role },
        { id: 'status', label: 'Status', cell: user => <div className="flex flex-col gap-1"><Badge variant={user.banned ? 'destructive' : 'outline'}>{user.banned ? 'Banned' : 'Active'}</Badge>{user.banned && user.ban_reason ? <p className="max-w-xs break-words text-xs text-muted-foreground">{user.ban_reason}</p> : null}{user.banned && user.ban_expires_at ? <time className="text-xs text-muted-foreground">Until {new Date(user.ban_expires_at).toLocaleString()}</time> : null}</div> },
        { id: 'actions', label: 'Actions', cell: user => <div className="flex gap-2">
          <Button variant="outline" disabled={busy} onClick={() => setDetailId(user.id)}>Details</Button>
          <DropdownMenu><DropdownMenuTrigger asChild><Button variant="outline" disabled={busy} aria-label={`Actions for ${user.email}`}>Actions</Button></DropdownMenuTrigger><DropdownMenuContent align="end" className="w-48"><DropdownMenuGroup>
          {user.system_role !== 'admin' && user.id !== identity.user_id ? <DropdownMenuItem disabled={busy} onSelect={() => open({ title: user.banned ? `Unban ${user.name}?` : `Ban ${user.name}?`, description: user.banned ? 'Allow this user to sign in again.' : 'Revokes active sessions. Organization owners must transfer ownership first.', path: `admin/users/${user.id}/ban`, kind: user.banned ? 'confirm' : 'ban', body: { banned: false } })}>{user.banned ? 'Unban' : 'Ban'}</DropdownMenuItem> : null}
          <DropdownMenuItem disabled={busy || user.banned} onSelect={() => open({ title: `Change platform role for ${user.name}?`, description: `Set platform role to ${user.system_role === 'admin' ? 'user' : 'admin'}. Organization roles are unchanged. The last active administrator is protected.`, path: `admin/users/${user.id}/role`, kind: 'confirm', body: { role: user.system_role === 'admin' ? 'user' : 'admin' } })}>{user.system_role === 'admin' ? 'Make user' : 'Make admin'}</DropdownMenuItem>
          <DropdownMenuItem disabled={busy} onSelect={() => open({ title: `Revoke sessions for ${user.name}?`, description: 'Revokes this user’s sessions, except your current session if this is your account.', path: `admin/users/${user.id}/revoke-sessions`, kind: 'confirm' })}>Revoke sessions</DropdownMenuItem>
          <DropdownMenuItem disabled={busy} onSelect={() => open({ title: `Revoke API keys for ${user.name}?`, description: 'Immediately invalidates all of this user’s API keys across organizations. Browser sessions remain unchanged.', path: `admin/users/${user.id}/revoke-api-keys`, kind: 'confirm' })}>Revoke API keys</DropdownMenuItem>
          <DropdownMenuItem disabled={busy || user.banned} onSelect={() => open({ title: `Reset password for ${user.name}?`, description: 'Generate a single-use reset token to share privately. MFA remains required.', path: `admin/users/${user.id}/password-reset`, kind: 'confirm' })}>Reset password</DropdownMenuItem>
          {user.id !== identity.user_id ? <DropdownMenuItem variant="destructive" disabled={busy} onSelect={() => open({ title: `Delete ${user.name}?`, description: 'Permanently removes the account, memberships, sessions and invitations issued by this account. Organization owners must transfer ownership first.', path: `admin/users/${user.id}/delete`, kind: 'delete', confirmation: user.email })}>Delete</DropdownMenuItem> : null}
          </DropdownMenuGroup></DropdownMenuContent></DropdownMenu>
        </div> },
      ]} />
    </Section>
    <Dialog open={!!detailId} onOpenChange={open => { if (!open) setDetailId(null) }}><DialogContent className="max-h-[calc(100svh-2rem)] overflow-y-auto sm:max-w-3xl"><DialogHeader><DialogTitle>User details</DialogTitle><DialogDescription>Account membership and up to 100 active sessions. Authentication secrets are never shown.</DialogDescription></DialogHeader>{detailError ? <Notice message={detailError} /> : !detail ? <p role="status">Loading user…</p> : <>
      <p>{detail.user.name} · {detail.user.email}</p><p className="text-sm text-muted-foreground">Platform role: {detail.user.system_role}</p>
      <DataTable label="User organizations" data={detail.organizations} columns={[{ id: 'name', label: 'Organization', value: org => org.name }, { id: 'role', label: 'Role', value: org => org.role }]} />
      <DataTable label="User sessions" data={detail.sessions} columns={[{ id: 'device', label: 'Device', value: session => session.user_agent || 'Unknown device' }, { id: 'ip', label: 'IP address', value: session => session.ip_address }, { id: 'current', label: 'Status', cell: session => session.current ? <Badge variant="outline">Protected</Badge> : 'Active' }]} />
    </>}</DialogContent></Dialog>
    <OperationDialog operation={operation} busy={busy} message={message} close={() => setOperation(null)} submit={async body => { if (!operation) return; const result = await action(operation.path, body); if (result) { if (result.token) setToken(result.token); setOperation(null) } }} />
  </div>
}

export function TeamPage({ identity, members, invitations, refresh }: { identity: Identity; members: Member[]; invitations: Invitation[]; refresh: Refresh }) {
  const manager = identity.role === 'owner' || identity.role === 'admin'
  const [profile, setProfile] = useState<OrgProfile | null>(null); const [profileError, setProfileError] = useState(''); const [version, setVersion] = useState(0)
  const [token, setToken] = useState(''); const [invitationRole, setInvitationRole] = useState('member'); const [transferTo, setTransferTo] = useState('')
  const [inviting, setInviting] = useState(false)
  const [invitationToken, setInvitationToken] = useState('')
  const [operation, setOperation] = useState<Operation | null>(null)
  const { busy, message, setMessage, action } = useActions(async () => { await refresh(); setVersion(value => value + 1) })
  useEffect(() => {
    // This page is keyed by organization. Keep its editor mounted during background refreshes.
    const abort = new AbortController(); setProfileError('')
    request<OrgProfile>('/auth/organizations/current', { signal: abort.signal }).then(value => { if (!abort.signal.aborted) setProfile(value) }).catch(error => { if (!abort.signal.aborted) setProfileError(error.message) })
    return () => abort.abort()
  }, [identity.organization_id, version])
  // A regenerated token is shown only in this browser until explicitly dismissed.
  const open = (value: Operation) => { setMessage(''); setOperation(value) }
  return <div className="flex flex-col gap-6"><h1 className="text-2xl font-semibold">Organization</h1>{!inviting && !operation ? <Notice message={message} /> : null}
    <Section title="Organization profile" description="Owners and organization admins can update the name and logo.">
      {profileError ? <><Notice message={profileError} /><Button variant="outline" onClick={() => setVersion(value => value + 1)}>Retry organization</Button></> : null}
      {!profile ? !profileError ? <p role="status">Loading organization…</p> : null : <>
        {profile.logo_url ? <img src={profile.logo_url} alt={`${profile.name} logo`} className="size-12 object-contain" referrerPolicy="no-referrer" /> : null}
        {manager ? <form key={`${profile.id}:${profile.name}:${profile.logo_url}`} onSubmit={async event => { const input = values(event); await action('organizations/current', { name: input.name, logo_url: input.logo_url.trim() || null }) }} className="flex flex-col gap-4"><fieldset disabled={busy}><FieldGroup><Text name="name" label="Organization name" maxLength={80} defaultValue={profile.name} /><Text name="logo_url" label="Logo URL" type="url" maxLength={2048} required={false} defaultValue={profile.logo_url ?? ''} /><FieldDescription>Optional HTTPS image URL. The image provider receives requests from your browser.</FieldDescription></FieldGroup></fieldset><Button type="submit" disabled={busy}>Save organization</Button></form> : <p>{profile.name}</p>}
      </>}
    </Section>
    <Section title="Members" description="Organization roles remain owner, admin and member."><DataTable label="Members" data={members} columns={[
      { id: 'name', label: 'Member', value: member => `${member.name} ${member.email}`, cell: member => <div><p>{member.name}{member.id === identity.user_id ? ' (you)' : ''}</p><p className="text-xs text-muted-foreground">{member.email}</p></div> },
      { id: 'role', label: 'Role', value: member => member.role },
      ...(manager ? [{ id: 'actions', label: 'Actions', cell: (member: Member) => <div className="flex flex-wrap gap-2">{identity.role === 'owner' ? ['owner', 'admin', 'member'].filter(role => role !== member.role).map(role => <Button key={role} variant="outline" disabled={busy} onClick={() => open({ title: `Change ${member.name} to ${role}?`, description: 'The last owner is protected. Platform access is unchanged.', path: `members/${member.id}/role`, kind: 'confirm', body: { role } })}>Make {role}</Button>) : null}{member.id !== identity.user_id && (identity.role === 'owner' || member.role === 'member') ? <Button variant="outline" disabled={busy} onClick={() => open({ title: `Remove ${member.name}?`, description: 'Removes membership and sessions active in this organization. Other memberships remain.', path: `members/${member.id}/remove`, kind: 'confirm' })}>Remove</Button> : null}</div> }] : []),
    ]} /></Section>
    {manager ? <Section title="Invitations" description="Tokens are bound to an email and expire after two days. Share generated tokens privately; email delivery is not configured.">
      <FormDialog open={inviting} onOpenChange={open => { setInviting(open); setMessage(''); if (open) setInvitationRole('member') }} title={invitationToken ? 'Save invitation token' : 'Invite member'} description={invitationToken ? 'Share this token privately with the invited person. It expires after two days.' : 'Invite a member to this organization. Email delivery is not configured.'} busy={busy || !!invitationToken} trigger={<Button className="self-start" disabled={busy}>Invite member</Button>}>
        <Notice message={message} />
        {invitationToken ? <><code className="break-all select-all rounded-md bg-muted p-3" aria-label="New invitation token">{invitationToken}</code><Button autoFocus onClick={() => { setInvitationToken(''); setInviting(false) }}>I saved the invitation token</Button></> : <form className="flex flex-col gap-4" onSubmit={async event => { const input = values(event); const result = await action('invitations', { ...input, role: invitationRole }); if (result?.token) setInvitationToken(result.token) }}>
          <fieldset disabled={busy}><FieldGroup><Text name="email" label="Invitation email" type="email" /><Choice label="Invitation role" value={invitationRole} onChange={setInvitationRole} options={identity.role === 'owner' ? [['member', 'Member'], ['admin', 'Admin']] : [['member', 'Member']]} /></FieldGroup></fieldset>
          <DialogFooter><Button type="button" variant="outline" disabled={busy} onClick={() => { setInviting(false); setMessage('') }}>Cancel</Button><Button type="submit" disabled={busy}>{busy ? 'Inviting…' : 'Invite member'}</Button></DialogFooter>
        </form>}
      </FormDialog>
      {token ? <Alert><AlertTitle>Invitation token</AlertTitle><AlertDescription><code className="break-all select-all">{token}</code><Button type="button" variant="outline" onClick={() => setToken('')}>Dismiss token</Button></AlertDescription></Alert> : null}
      <DataTable label="Invitations" data={invitations} columns={[
        { id: 'email', label: 'Email', value: invite => invite.email }, { id: 'role', label: 'Role', value: invite => invite.role }, { id: 'expires', label: 'Expires', value: invite => invite.expires_at, cell: invite => <time dateTime={invite.expires_at}>{new Date(invite.expires_at).toLocaleString()}</time> },
        { id: 'actions', label: 'Actions', cell: invite => <div className="flex flex-wrap gap-2">{invite.role === 'member' || identity.role === 'owner' ? <Button variant="outline" disabled={busy} onClick={() => open({ title: `Regenerate invitation for ${invite.email}?`, description: 'Invalidates the old token and creates a new token valid for two days.', path: `invitations/${invite.id}/renew`, kind: 'confirm' })}>Regenerate</Button> : null}<Button variant="outline" disabled={busy || (invite.role === 'admin' && identity.role !== 'owner')} onClick={() => open({ title: `Cancel invitation for ${invite.email}?`, description: 'The invitation token will stop working.', path: `invitations/${invite.id}/revoke`, kind: 'confirm' })}>Cancel invitation</Button></div> },
      ]} />
    </Section> : null}
    {identity.role === 'owner' ? <Section title="Transfer ownership" description="Promotes another active member to owner and changes your organization role to admin.">
      <Choice label="New owner" value={transferTo} onChange={setTransferTo} disabled={busy} options={[[ '', 'Select a member' ], ...members.filter(member => member.id !== identity.user_id).map(member => [member.id, `${member.name} (${member.email})`] as [string, string])]} />
      <Button variant="outline" disabled={busy || !members.some(member => member.id === transferTo && member.id !== identity.user_id)} onClick={() => open({ title: 'Transfer organization ownership?', description: `Make ${members.find(member => member.id === transferTo)?.email} an owner. Your role becomes admin.`, path: 'organizations/transfer', kind: 'confirm', body: { user_id: transferTo } })}>Transfer ownership</Button>
    </Section> : null}
    <Section title="Organization access" description="After deletion, your current session switches to another organization you belong to, preferring the default. If none remains, you are signed out. Other sessions using the deleted organization are signed out. Leaving still signs out sessions using this organization.">
      <div className="flex flex-wrap gap-2"><Button variant="outline" disabled={busy} onClick={() => open({ title: 'Leave organization?', description: 'Removes your membership. A sole owner must transfer ownership first.', path: 'organizations/leave', kind: 'confirm' })}>Leave organization</Button>
      {identity.role === 'owner' ? <Button variant="destructive" disabled={busy || !profile || profile.is_default} title={profile?.is_default ? 'The default organization cannot be deleted' : undefined} onClick={() => open({ title: 'Delete organization?', description: 'Permanently deletes this organization, its memberships and invitations. All projects must be deleted first. User accounts are retained. Your session switches to another organization you belong to, preferring the default, or signs out if none remains.', path: 'organizations/delete', kind: 'delete', confirmation: profile?.name })}>Delete organization</Button> : null}</div>
      {profile?.is_default ? <p className="text-sm text-muted-foreground">The default organization cannot be deleted.</p> : null}
    </Section>
    <OperationDialog operation={operation} busy={busy} message={message} close={() => setOperation(null)} submit={async body => { if (!operation) return; const result = await action(operation.path, body); if (result) { if (result.token) setToken(result.token); setOperation(null) } }} />
  </div>
}
