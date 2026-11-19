import { useId, useState } from 'react'
import { Link } from '@tanstack/react-router'
import type { FormEvent } from 'react'
import { ChevronDown, Eye, EyeOff, Layers3 } from 'lucide-react'
import { InputGroup, InputGroupInput, InputGroupAddon, InputGroupButton } from '@/components/ui/input-group'
import { Collapsible, CollapsibleContent, CollapsibleTrigger } from '@/components/ui/collapsible'
import { Button } from '@/components/ui/button'
import { Input } from '@/components/ui/input'
import { Field, FieldGroup, FieldLabel } from '@/components/ui/field'
import { Alert, AlertTitle, AlertDescription } from '@/components/ui/alert'
import { Badge } from '@/components/ui/badge'
import { Table, TableHeader, TableHead, TableBody, TableRow, TableCell } from '@/components/ui/table'
import { DataTable } from '@/components/data-table'
import { request } from '@/lib/api'

export type Identity = { hosting_student?: boolean; session_id: string; user_id: string; name: string; email: string; system_role: string; organization_id: string; organization_name: string; role: string }
export type Session = { id: string; created_at: string; expires_at: string; user_agent: string; ip_address: string; current: boolean }
export type Member = { id: string; name: string; email: string; role: string }
export type Organization = { id: string; name: string; role: string }
export type Invitation = { id: string; email: string; role: string; expires_at: string }
export type User = { id: string; name: string; email: string; system_role: string; banned: boolean }
export type Security = { two_factor_enabled: boolean; recovery_codes_remaining: number; fresh_until: string }
export type Audit = { id: string; action: string; created_at: string }
type Refresh = () => Promise<void>

function Message({ message }: { message: string }) { return message ? <Alert><AlertTitle>Account notice</AlertTitle><AlertDescription>{message}</AlertDescription></Alert> : null }
function TextField({ name, label, type = 'text', required = true, autoComplete }: { name: string; label: string; type?: string; required?: boolean; autoComplete?: string }) { const id = useId(); return <Field><FieldLabel htmlFor={id}>{label}</FieldLabel><Input id={id} name={name} type={type} required={required} autoComplete={autoComplete} maxLength={type === 'password' ? 1024 : 254} /></Field> }
function data(event: FormEvent<HTMLFormElement>): Record<string, string> { event.preventDefault(); return Object.fromEntries(new FormData(event.currentTarget).entries()) as Record<string, string> }
function errorText(error: unknown) { return error instanceof Error ? error.message : 'Account operation failed.' }
const panel = 'flex flex-col gap-4 rounded-xl border bg-card p-6 text-card-foreground'

function LoginPassword({ name, label, newPassword }: { name: string; label: string; newPassword: boolean }) {
  const id = useId()
  const [visible, setVisible] = useState(false)
  return <Field>
    <FieldLabel htmlFor={id}>{label}</FieldLabel>
    <InputGroup>
      <InputGroupInput id={id} name={name} type={visible ? 'text' : 'password'} autoComplete={newPassword ? 'new-password' : 'current-password'} required maxLength={1024} minLength={newPassword ? 12 : undefined} />
      <InputGroupAddon align="inline-end"><InputGroupButton type="button" size="icon-sm" aria-label={visible ? 'Hide password' : 'Show password'} aria-pressed={visible} onClick={() => setVisible(!visible)}>{visible ? <EyeOff /> : <Eye />}</InputGroupButton></InputGroupAddon>
    </InputGroup>
  </Field>
}

export function LoginPage({ setupRequired, refresh, initialMode = 'login' }: { setupRequired: boolean; refresh: Refresh; initialMode?: 'login' | 'signup' }) {
  const [invitation, setInvitation] = useState(initialMode === 'signup' && !setupRequired)
  const [reset, setReset] = useState(false)
  const [showCode, setShowCode] = useState(false)
  const [moreOptions, setMoreOptions] = useState(false)
  const [busy, setBusy] = useState(false)
  const [message, setMessage] = useState('')
  const mode = setupRequired ? 'setup' : reset ? 'reset' : invitation ? 'invitation' : 'login'
  const title = setupRequired ? 'Create your account' : reset ? 'Reset password' : invitation ? 'Accept invitation' : 'Sign in'
  const description = setupRequired ? 'The first account becomes the platform administrator.' : reset ? 'Use the reset token provided by your administrator.' : invitation ? 'Enter your invitation details to join your organization.' : 'Enter your email and password to sign in'
  async function submit(event: FormEvent<HTMLFormElement>) {
    const values = data(event); setBusy(true); setMessage('')
    try {
      await request(`/auth/${setupRequired ? 'setup' : reset ? 'reset-password' : invitation ? 'accept-invitation' : 'sign-in'}`, { method: 'POST', headers: { 'Content-Type': 'application/json' }, body: JSON.stringify(values) })
      if (reset) { setReset(false); setMessage('Password reset. Sign in with your new password.') }
      await refresh()
    } catch (error) { setMessage(errorText(error)) } finally { setBusy(false) }
  }
  return <main className="flex min-h-svh items-center justify-center bg-background px-6 py-12">
    <section className="flex w-full max-w-sm flex-col gap-6" aria-labelledby="login-heading">
      <header className="flex flex-col items-center gap-3 text-center">
        <Link to="/" aria-label="Ferrix home" title="Ferrix home" className="mb-3 flex size-12 items-center justify-center rounded-xl border bg-card outline-none focus-visible:ring-2 focus-visible:ring-ring focus-visible:ring-offset-4 focus-visible:ring-offset-background"><Layers3 className="size-6" aria-hidden="true" /></Link>
        <h1 id="login-heading" className="text-2xl font-semibold tracking-tight">{title}</h1>
        <p className="text-sm text-muted-foreground">{description}</p>
      </header>
      <Message message={message} />
      <form key={mode} onSubmit={submit} className="flex flex-col gap-6">
        <fieldset disabled={busy}><FieldGroup>
          {setupRequired || invitation ? <Field><FieldLabel htmlFor="login-name">Name</FieldLabel><Input id="login-name" name="name" required maxLength={80} autoComplete="name"  /></Field> : null}
          {!reset ? <Field><FieldLabel htmlFor="login-email">Email</FieldLabel><Input id="login-email" name="email" required type="email" maxLength={254} autoComplete="username"  /></Field> : null}
          <LoginPassword name={reset ? 'new_password' : 'password'} label={reset ? 'New password' : invitation ? 'Password (existing account password, or a new password)' : 'Password'} newPassword={setupRequired || reset} />
          {invitation || reset ? <TextField name="token" label={reset ? 'Password reset token' : 'Invitation token'} type="password" autoComplete="off" /> : null}
          {!setupRequired ? <Collapsible open={showCode} onOpenChange={setShowCode}><CollapsibleTrigger asChild><Button type="button" variant="link">{showCode ? 'Hide verification code' : 'Use two-factor authentication'}</Button></CollapsibleTrigger><CollapsibleContent className="pt-4"><TextField name="code" label="Authenticator or recovery code" required={false} autoComplete="one-time-code" /></CollapsibleContent></Collapsible> : null}
        </FieldGroup></fieldset>
        <Button type="submit" className="w-full" disabled={busy}>{busy ? 'Please wait…' : setupRequired ? 'Create account' : reset ? 'Reset password' : invitation ? 'Accept invitation' : 'Login'}</Button>
      </form>
      {!setupRequired && (invitation || reset) ? <Button variant="link" disabled={busy} onClick={() => {setInvitation(false);setReset(false);setShowCode(false);setMessage('')}}>Back to sign in</Button> : null}
      {!setupRequired && !invitation && !reset ? <Collapsible open={moreOptions} onOpenChange={setMoreOptions} className="flex flex-col items-center gap-3"><CollapsibleTrigger asChild><Button variant="ghost" disabled={busy}>More sign-in options<ChevronDown data-icon="inline-end" /></Button></CollapsibleTrigger><CollapsibleContent className="flex flex-col items-center gap-1"><Button variant="link" disabled={busy} onClick={() => {setInvitation(true);setShowCode(false);setMessage('')}}>Accept a team invitation</Button><Button variant="link" disabled={busy} onClick={() => {setReset(true);setShowCode(false);setMessage('')}}>Reset password</Button><p className="max-w-xs text-center text-xs leading-relaxed text-muted-foreground">Your administrator provides invitation and password reset tokens.</p></CollapsibleContent></Collapsible> : null}

    </section>
  </main>
}

export function SecurityPage({ sessions, security, audit, refresh }: { sessions: Session[]; security: Security; audit: Audit[]; refresh: Refresh }) {
  const [message, setMessage] = useState(''); const [busy, setBusy] = useState(false)
  const [enrollment, setEnrollment] = useState<{ secret: string; otpauth_url: string } | null>(null)
  const [codes, setCodes] = useState<string[]>([])
  async function action(path: string, body = {}) { setBusy(true); setMessage(''); try { const result = await request<unknown>(`/auth/${path}`, { method: 'POST', headers: { 'Content-Type': 'application/json' }, body: JSON.stringify(body) }); await refresh(); return result } catch (error) { setMessage(errorText(error)); return null } finally { setBusy(false) } }
  async function enroll(event: FormEvent<HTMLFormElement>) { const values = data(event); const result = await action('mfa/enroll', values); if (result) setEnrollment(result as { secret: string; otpauth_url: string }) }
  async function confirm(event: FormEvent<HTMLFormElement>) { const result = await action('mfa/confirm', data(event)); if (result) { setCodes((result as { recovery_codes: string[] }).recovery_codes); setEnrollment(null) } }
  return <div className="flex flex-col gap-6"><h1 className="text-2xl font-semibold">Security and sessions</h1><Message message={message} />
    <section className={panel}><h2 className="font-semibold">Verify your identity</h2><p className="text-sm text-muted-foreground">Sensitive actions need a password verification within the last 15 minutes. Authenticator codes can be used only once per time step.</p><form className="flex flex-col gap-4" onSubmit={async e => { await action('reauthenticate', data(e)) }}><FieldGroup><TextField name="password" label="Current password" type="password" autoComplete="current-password" /><TextField name="code" label="Authenticator or recovery code (if enabled)" required={false} autoComplete="one-time-code" /></FieldGroup><Button type="submit" disabled={busy}>Verify identity</Button></form></section>
    <section className={panel}><h2 className="font-semibold">Active sessions</h2><p className="text-sm text-muted-foreground">Sessions expire after three days of inactivity, with an absolute limit of 30 days.</p><DataTable label="Sessions" data={sessions} columns={[
      { id: 'device', label: 'Device', value: session => `${session.user_agent} ${session.ip_address} ${session.current ? 'Current' : ''}`, cell: session => <div className="flex flex-col gap-1"><p className="max-w-sm truncate" title={session.user_agent}>{session.user_agent || 'Unknown device'}</p><span className="text-xs text-muted-foreground">{session.ip_address}</span>{session.current ? <Badge variant="secondary" className="self-start">Current</Badge> : null}</div> },
      { id: 'expires', label: 'Expires', value: session => session.expires_at, cell: session => <time dateTime={session.expires_at}>{new Date(session.expires_at).toLocaleString()}</time> },
      { id: 'actions', label: 'Action', cell: session => <Button variant="outline" disabled={busy} onClick={() => action(`sessions/${session.id}/revoke`)}>Revoke</Button> },
    ]} /><Button variant="outline" disabled={busy} onClick={() => action('sessions/revoke-others')}>Revoke other sessions</Button></section>
    <section className={panel}><h2 className="font-semibold">Two-factor authentication</h2><p className="text-sm text-muted-foreground">{security.two_factor_enabled ? `Enabled. ${security.recovery_codes_remaining} recovery codes remain.` : 'Add an authenticator app to protect your account.'}</p>
      <form className="flex flex-col gap-4" onSubmit={security.two_factor_enabled ? async e => { await action('mfa/disable', data(e)) } : enroll}><FieldGroup><TextField name="password" label="Confirm password" type="password" autoComplete="current-password" />{security.two_factor_enabled ? <TextField name="code" label="Authenticator or recovery code" autoComplete="one-time-code" /> : null}</FieldGroup><Button type="submit" variant="outline" disabled={busy}>{security.two_factor_enabled ? 'Disable two-factor authentication' : 'Set up authenticator'}</Button></form>
      {enrollment ? <div className="flex flex-col gap-4"><p className="text-sm">Enter this setup key in your authenticator app. Enrollment expires in 10 minutes.</p><code className="break-all rounded-md bg-muted p-3">{enrollment.secret}</code><form onSubmit={confirm} className="flex flex-col gap-4"><FieldGroup><TextField name="code" label="Code from authenticator" autoComplete="one-time-code" /></FieldGroup><Button type="submit" disabled={busy}>Enable two-factor authentication</Button></form></div> : null}
      {codes.length ? <Alert><AlertTitle>Save your recovery codes now</AlertTitle><AlertDescription><p>Each code works once. These codes will not be shown again.</p><pre className="max-w-full overflow-auto py-3">{codes.join('\n')}</pre><Button variant="outline" onClick={() => setCodes([])}>I saved my codes</Button></AlertDescription></Alert> : null}
    </section>
    <section className={panel}><h2 className="font-semibold">Change password</h2><p className="text-sm text-muted-foreground">Use at least 12 characters. Changing your password signs out every session, including this one.</p><form onSubmit={async e => { await action('change-password', data(e)) }} className="flex flex-col gap-4"><FieldGroup><TextField name="password" label="Existing password" type="password" autoComplete="current-password" /><TextField name="new_password" label="New password" type="password" autoComplete="new-password" /><TextField name="code" label="Authenticator or recovery code (if enabled)" required={false} autoComplete="one-time-code" /></FieldGroup><Button disabled={busy} type="submit">Change password and sign out</Button></form></section>
    {audit.length ? <section className={panel}><h2 className="font-semibold">Organization security activity</h2>{audit.map(item => <p key={item.id} className="text-sm">{new Date(item.created_at).toLocaleString()} — {item.action}</p>)}</section> : null}
  </div>
}

export function TeamPage({ identity, organizations, members, invitations, refresh }: { identity: Identity; organizations: Organization[]; members: Member[]; invitations: Invitation[]; refresh: Refresh }) {
  const [message, setMessage] = useState(''); const [token, setToken] = useState(''); const [busy, setBusy] = useState(false)
  async function action(path: string, body = {}) { setBusy(true); setMessage(''); try { const result = await request<{ token?: string }>(`/auth/${path}`, { method: 'POST', headers: { 'Content-Type': 'application/json' }, body: JSON.stringify(body) }); if (result?.token) setToken(result.token); await refresh() } catch (error) { setMessage(errorText(error)) } finally { setBusy(false) } }
  const manager = identity.role === 'owner' || identity.role === 'admin'
  return <div className="flex flex-col gap-6"><h1 className="text-2xl font-semibold">Organization</h1><Message message={message} />
    <section className={panel}><h2 className="font-semibold">Your organizations</h2><div className="flex flex-wrap gap-2">{organizations.map(org => <Button key={org.id} variant={org.id === identity.organization_id ? 'secondary' : 'outline'} disabled={busy || org.id === identity.organization_id} onClick={() => action('organizations/switch', { organization_id: org.id })}>{org.name} ({org.role})</Button>)}</div>{identity.system_role === 'admin' ? <form onSubmit={async e => { await action('organizations', data(e)) }} className="flex flex-col gap-4"><FieldGroup><TextField name="name" label="New organization name" /></FieldGroup><Button type="submit" disabled={busy}>Create organization</Button></form> : null}</section>
    <section className={panel}><h2 className="font-semibold">Members</h2><Table><TableHeader><TableRow><TableHead>Name</TableHead><TableHead>Role</TableHead>{manager ? <TableHead>Actions</TableHead> : null}</TableRow></TableHeader><TableBody>{members.map(member => <TableRow key={member.id}><TableCell>{member.name}<p className="text-xs text-muted-foreground">{member.email}</p></TableCell><TableCell>{member.role}</TableCell>{manager ? <TableCell><div className="flex flex-wrap gap-2">{identity.role === 'owner' ? ['owner', 'admin', 'member'].filter(role => role !== member.role).map(role => <Button key={role} size="sm" variant="outline" disabled={busy} onClick={() => action(`members/${member.id}/role`, { role })}>Make {role}</Button>) : null}<Button size="sm" variant="outline" disabled={busy} onClick={() => action(`members/${member.id}/remove`)}>Remove</Button></div></TableCell> : null}</TableRow>)}</TableBody></Table></section>
    {manager ? <section className={panel}><h2 className="font-semibold">Invite a member</h2><p className="text-sm text-muted-foreground">Share the generated token privately. It expires in two days and is bound to this email address.</p><form onSubmit={async e => { const values = data(e); const submitter = (e.nativeEvent as SubmitEvent).submitter as HTMLButtonElement | null; await action('invitations', { ...values, role: submitter?.value || 'member' }) }} className="flex flex-col gap-4"><FieldGroup><TextField name="email" label="Invitation email" type="email" /></FieldGroup><div className="flex gap-2"><Button type="submit" value="member" disabled={busy}>Invite member</Button>{identity.role === 'owner' ? <Button type="submit" value="admin" variant="outline" disabled={busy}>Invite admin</Button> : null}</div></form>{token ? <Alert><AlertTitle>Invitation token</AlertTitle><AlertDescription><code className="break-all">{token}</code></AlertDescription></Alert> : null}{invitations.map(invite => <div key={invite.id} className="flex flex-wrap items-center justify-between gap-2 text-sm"><span>{invite.email} ({invite.role})</span><Button variant="outline" disabled={busy} onClick={() => action(`invitations/${invite.id}/revoke`)}>Revoke invitation</Button></div>)}</section> : null}
  </div>
}

export function AdminPage({ users, refresh }: { users: User[]; refresh: Refresh }) {
  const [message, setMessage] = useState(''); const [busy, setBusy] = useState(false); const [resetToken, setResetToken] = useState('')
  async function action(user: User, path: string, body = {}) { setBusy(true); setMessage(''); try { const result = await request<{ token?: string }>(`/auth/admin/users/${user.id}/${path}`, { method: 'POST', headers: { 'Content-Type': 'application/json' }, body: JSON.stringify(body) }); if (result?.token) setResetToken(result.token); await refresh() } catch (error) { setMessage(errorText(error)) } finally { setBusy(false) } }
  return <div className="flex flex-col gap-6"><h1 className="text-2xl font-semibold">User administration</h1><Message message={message} />{resetToken ? <Alert><AlertTitle>Password reset token</AlertTitle><AlertDescription><p>Share privately with the account owner. Expires in 15 minutes; two-factor authentication remains required.</p><code className="break-all">{resetToken}</code><Button variant="outline" onClick={() => setResetToken('')}>Dismiss token</Button></AlertDescription></Alert> : null}<section className={panel}><DataTable label="Users" data={users} columns={[
      { id: 'user', label: 'User', value: user => `${user.name} ${user.email}`, cell: user => <div><p>{user.name}</p><p className="text-xs text-muted-foreground">{user.email}</p></div> },
      { id: 'role', label: 'Role', value: user => user.system_role },
      { id: 'status', label: 'Status', value: user => user.banned ? 'Banned' : 'Active', cell: user => <Badge variant={user.banned ? 'destructive' : 'outline'}>{user.banned ? 'Banned' : 'Active'}</Badge> },
      { id: 'actions', label: 'Actions', cell: user => <div className="flex flex-wrap gap-2">{user.system_role !== 'admin' ? <Button variant="outline" disabled={busy} onClick={() => action(user, 'ban', { banned: !user.banned })}>{user.banned ? 'Unban' : 'Ban'}</Button> : null}<Button variant="outline" disabled={busy} onClick={() => action(user, 'revoke-sessions')}>Revoke sessions</Button><Button variant="outline" disabled={busy || user.banned} onClick={() => action(user, 'password-reset')}>Issue password reset</Button></div> },
    ]} /></section></div>
}
