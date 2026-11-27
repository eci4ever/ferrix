import { useId, useRef, useState } from 'react'
import { Link } from '@tanstack/react-router'
import type { FormEvent, ReactNode } from 'react'
import { Eye, EyeOff, Layers3 } from 'lucide-react'
import { InputGroup, InputGroupInput, InputGroupAddon, InputGroupButton } from '@/components/ui/input-group'
import { Collapsible, CollapsibleContent, CollapsibleTrigger } from '@/components/ui/collapsible'
import { Button } from '@/components/ui/button'
import { Input } from '@/components/ui/input'
import { Field, FieldGroup, FieldLabel } from '@/components/ui/field'
import { Alert, AlertTitle, AlertDescription } from '@/components/ui/alert'
import { Badge } from '@/components/ui/badge'
import { Card, CardHeader, CardTitle, CardDescription, CardContent } from '@/components/ui/card'
import { Separator } from '@/components/ui/separator'
import { Dialog, DialogContent, DialogHeader, DialogTitle, DialogDescription, DialogTrigger } from '@/components/ui/dialog'
import { Tabs, TabsList, TabsTrigger, TabsContent } from '@/components/ui/tabs'
import { DataTable } from '@/components/data-table'
import { ApiRequestError, request } from '@/lib/api'
import { QRCodeSVG } from 'qrcode.react'
import { toast } from 'sonner'

export type Identity = { session_id: string; user_id: string; name: string; email: string; system_role: string; organization_id: string; organization_name: string; role: string }
export type Session = { id: string; created_at: string; expires_at: string; user_agent: string; ip_address: string; current: boolean }
export type Member = { id: string; name: string; email: string; role: string }
export type Organization = { id: string; name: string; role: string; logo_url?: string | null }
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

function LoginPassword({ name, label, newPassword, action }: { name: string; label: string; newPassword: boolean; action?: ReactNode }) {
  const id = useId()
  const [visible, setVisible] = useState(false)
  return <Field>
    <div className="flex items-center justify-between gap-2"><FieldLabel htmlFor={id}>{label}</FieldLabel>{action}</div>
    <InputGroup>
      <InputGroupInput id={id} name={name} type={visible ? 'text' : 'password'} autoComplete={newPassword ? 'new-password' : 'current-password'} required maxLength={1024} minLength={newPassword ? 12 : undefined} />
      <InputGroupAddon align="inline-end"><InputGroupButton type="button" size="icon-sm" aria-label={visible ? 'Hide password' : 'Show password'} aria-pressed={visible} onClick={() => setVisible(!visible)}>{visible ? <EyeOff /> : <Eye />}</InputGroupButton></InputGroupAddon>
    </InputGroup>
  </Field>
}

function VerificationCode({ recovery = false }: { recovery?: boolean }) {
  const id = useId()
  return <Field>
    <FieldLabel htmlFor={id}>{recovery ? 'Recovery code' : 'Authenticator code'}</FieldLabel>
    <Input id={id} name="code" type="text" autoComplete="one-time-code" autoFocus required
      inputMode={recovery ? 'text' : 'numeric'} pattern={recovery ? undefined : '[0-9]{6}'}
      maxLength={recovery ? 64 : 6} minLength={recovery ? undefined : 6} spellCheck={false} autoCapitalize="none" />
  </Field>
}

function LoginVerification({ busy, submit, cancel }: { busy: boolean; submit: (event: FormEvent<HTMLFormElement>) => Promise<void>; cancel: () => Promise<void> }) {
  const [recovery, setRecovery] = useState(false)
  return <>
    <form onSubmit={submit} className="flex flex-col gap-6">
      <fieldset disabled={busy}><FieldGroup><VerificationCode key={String(recovery)} recovery={recovery} /></FieldGroup></fieldset>
      <p className="text-sm text-muted-foreground">{recovery ? 'Enter one of the recovery codes you saved. Each code can be used once.' : 'Enter the 6-digit code from your authenticator app. Use a new code if the previous one was already accepted.'} This sign-in request expires in five minutes.</p>
      <Button type="submit" disabled={busy}>{busy ? 'Verifying…' : 'Verify and sign in'}</Button>
    </form>
    <div className="flex flex-col items-center gap-1">
      <Button type="button" variant="link" disabled={busy} onClick={() => setRecovery(!recovery)}>{recovery ? 'Use authenticator app' : 'Use a recovery code'}</Button>
      <Button type="button" variant="ghost" disabled={busy} onClick={cancel}>Back to sign in</Button>
    </div>
  </>
}

export function LoginPage({ setupRequired, refresh, initialMode = 'login' }: { setupRequired: boolean; refresh: Refresh; initialMode?: 'login' | 'signup' }) {
  const [challenge, setChallenge] = useState(false)
  const invitation = initialMode === 'signup' && !setupRequired
  const [reset, setReset] = useState(false)
  const [showCode, setShowCode] = useState(false)
  const [busy, setBusy] = useState(false)
  const [message, setMessage] = useState('')
  const mode = setupRequired ? 'setup' : reset ? 'reset' : invitation ? 'invitation' : 'login'
  const title = challenge ? 'Two-factor authentication' : setupRequired ? 'Create your account' : reset ? 'Reset password' : invitation ? 'Create your account' : 'Welcome back'
  const description = challenge ? 'Verify your identity to finish signing in.' : setupRequired ? 'The first account becomes the platform administrator.' : reset ? 'Use the reset token provided by your administrator.' : invitation ? 'Use your team invitation to join your organization.' : 'Sign in to your Ferrix account'
  async function submit(event: FormEvent<HTMLFormElement>) {
    const values = data(event); setBusy(true); setMessage('')
    try {
      const result = await request<{ two_factor_required?: boolean }>(`/auth/${setupRequired ? 'setup' : reset ? 'reset-password' : invitation ? 'accept-invitation' : 'sign-in'}`, { method: 'POST', headers: { 'Content-Type': 'application/json' }, body: JSON.stringify(values) })
      if (result?.two_factor_required) { setChallenge(true); return }
      if (reset) { setReset(false); setMessage('Password reset. Sign in with your new password.') }
      await refresh()
    } catch (error) { setMessage(errorText(error)) } finally { setBusy(false) }
  }
  async function verifyLogin(event: FormEvent<HTMLFormElement>) {
    const values = data(event); setBusy(true); setMessage('')
    try {
      await request('/auth/sign-in/verify', { method: 'POST', headers: { 'Content-Type': 'application/json' }, body: JSON.stringify({ code: values.code.trim() }) })
      await refresh()
    } catch (error) {
      if (error instanceof ApiRequestError && error.status === 410) setChallenge(false)
      setMessage(errorText(error))
    } finally { setBusy(false) }
  }
  async function cancelLogin() {
    setBusy(true); setMessage('')
    try {
      await request('/auth/sign-in/cancel', { method: 'POST' })
      setChallenge(false)
    } catch (error) { setMessage(errorText(error)) } finally { setBusy(false) }
  }
  return <main className="flex min-h-svh items-center justify-center bg-muted/40 px-4 py-8 sm:px-6 sm:py-12">
    <section className="flex w-full max-w-sm flex-col gap-6" aria-labelledby="login-heading">
      <Link to="/" aria-label="Ferrix home" className="flex items-center justify-center gap-2 self-center rounded-md outline-none focus-visible:ring-2 focus-visible:ring-ring focus-visible:ring-offset-4 focus-visible:ring-offset-background">
        <span className="flex size-8 items-center justify-center rounded-lg bg-primary text-primary-foreground"><Layers3 className="size-5" aria-hidden="true" /></span><span className="text-lg font-semibold">Ferrix</span>
      </Link>
      <Card className="gap-6 py-6">
        <CardHeader className="gap-2 px-6 text-center"><CardTitle><h1 id="login-heading" className="text-2xl font-semibold tracking-tight">{title}</h1></CardTitle><CardDescription>{description}</CardDescription></CardHeader>
        <CardContent className="flex flex-col gap-6 px-6">
      <Message message={message} />
      {!challenge && mode === 'login' ? <>
        <div className="flex flex-col gap-3">
          <Button type="button" variant="outline" disabled={busy} onClick={() => setMessage('Google sign-in is not available yet. Use the form below.')}>
            <svg viewBox="0 0 24 24" fill="currentColor" aria-hidden="true" data-icon="inline-start"><path d="M21.6 12.23c0-.71-.06-1.39-.18-2.05H12v3.88h5.38a4.6 4.6 0 0 1-2 3.02v2.51h3.24c1.9-1.75 2.98-4.33 2.98-7.36ZM12 22c2.7 0 4.96-.9 6.62-2.41l-3.24-2.51c-.9.6-2.06.96-3.38.96-2.6 0-4.8-1.76-5.59-4.12H3.07v2.59A10 10 0 0 0 12 22ZM6.41 13.92a6 6 0 0 1 0-3.84V7.49H3.07a10 10 0 0 0 0 9.02l3.34-2.59ZM12 5.96c1.47 0 2.79.51 3.82 1.51l2.87-2.87A9.6 9.6 0 0 0 12 2a10 10 0 0 0-8.93 5.49l3.34 2.59C7.2 7.72 9.4 5.96 12 5.96Z" /></svg>
            Continue with Google
          </Button>
          <Button type="button" variant="outline" disabled={busy} onClick={() => setMessage('GitHub sign-in is not available yet. Use the form below.')}>
            <svg viewBox="0 0 24 24" fill="currentColor" aria-hidden="true" data-icon="inline-start"><path d="M12 .75a11.25 11.25 0 0 0-3.56 21.92c.56.1.77-.24.77-.54v-2.1c-3.13.68-3.79-1.33-3.79-1.33-.51-1.3-1.25-1.65-1.25-1.65-1.02-.7.08-.69.08-.69 1.13.08 1.73 1.16 1.73 1.16 1 1.72 2.63 1.22 3.27.93.1-.73.39-1.22.71-1.5-2.5-.29-5.13-1.25-5.13-5.56 0-1.23.44-2.23 1.16-3.02-.12-.29-.5-1.43.11-2.98 0 0 .95-.3 3.1 1.16a10.8 10.8 0 0 1 5.64 0c2.15-1.46 3.1-1.16 3.1-1.16.61 1.55.23 2.69.11 2.98.72.79 1.16 1.79 1.16 3.02 0 4.32-2.64 5.27-5.15 5.55.4.35.76 1.03.76 2.08v3.11c0 .3.2.65.78.54A11.25 11.25 0 0 0 12 .75Z" /></svg>
            Continue with GitHub
          </Button>
        </div>
        <div className="flex items-center gap-3"><Separator className="flex-1" /><span className="shrink-0 text-xs text-muted-foreground">Or continue with</span><Separator className="flex-1" /></div>
      </> : null}
      {challenge ? <LoginVerification busy={busy} submit={verifyLogin} cancel={cancelLogin} /> : <form key={mode} onSubmit={submit} className="flex flex-col gap-6">
        <fieldset disabled={busy}><FieldGroup>
          {setupRequired || invitation ? <Field><FieldLabel htmlFor="login-name">Name</FieldLabel><Input id="login-name" name="name" required maxLength={80} autoComplete="name"  /></Field> : null}
          {!reset ? <Field><FieldLabel htmlFor="login-email">Email</FieldLabel><Input id="login-email" name="email" required type="email" maxLength={254} autoComplete="username" placeholder="you@example.com" /></Field> : null}
          <LoginPassword name={reset ? 'new_password' : 'password'} label={reset ? 'New password' : 'Password'} newPassword={setupRequired || reset}
            action={!setupRequired && !invitation && !reset ? <Button type="button" variant="link" className="h-auto p-0" disabled={busy} onClick={() => {setReset(true);setShowCode(false);setMessage('')}}>Forgot your password?</Button> : undefined} />
          {invitation ? <p className="text-xs text-muted-foreground">Already have an account? Use your current password. New accounts need at least 12 characters.</p> : null}
          {invitation || reset ? <TextField name="token" label={reset ? 'Password reset token' : 'Invitation token'} type="password" autoComplete="off" /> : null}
          {!setupRequired && (invitation || reset) ? <Collapsible open={showCode} onOpenChange={setShowCode}><CollapsibleTrigger asChild><Button type="button" variant="link">{showCode ? 'Hide verification code' : 'Use two-factor authentication'}</Button></CollapsibleTrigger><CollapsibleContent className="pt-4"><TextField name="code" label="Authenticator or recovery code" required={false} autoComplete="one-time-code" /></CollapsibleContent></Collapsible> : null}
        </FieldGroup></fieldset>
        <Button type="submit" className="w-full" disabled={busy}>{busy ? 'Please wait…' : setupRequired ? 'Create account' : reset ? 'Reset password' : invitation ? 'Accept invitation' : 'Login'}</Button>
      </form>}
      {!challenge && reset ? <Button variant="link" disabled={busy} onClick={() => {setReset(false);setShowCode(false);setMessage('')}}>Back to sign in</Button> : null}
      {!challenge && !reset ? <p className="text-center text-sm text-muted-foreground">{setupRequired || invitation ? <>Already have an account? <Link to="/login" className="text-foreground underline underline-offset-4">Sign in</Link></> : <>Don't have an account? <Link to="/signup" className="text-foreground underline underline-offset-4">Sign up</Link></>}</p> : null}
        </CardContent>
      </Card>

    </section>
  </main>
}

export function SecurityPage({ sessions, security, audit, refresh }: { sessions: Session[]; security: Security; audit: Audit[]; refresh: Refresh }) {
  const [tab, setTab] = useState('verification')
  const tabClass = (value: string) => tab === value ? 'min-w-0' : 'hidden'
  const formClass = 'flex max-w-xl flex-col gap-4'
  const [message, setMessage] = useState(''); const [busy, setBusy] = useState(false)
  const [enrollment, setEnrollment] = useState<{ secret: string; otpauth_url: string } | null>(null)
  const [codes, setCodes] = useState<string[]>([])
  const [setupOpen, setSetupOpen] = useState(false)
  const mfaHeading = useRef<HTMLHeadingElement>(null)
  async function action(path: string, body = {}) {
    setBusy(true); setMessage('')
    try {
      const result = await request<unknown>(`/auth/${path}`, {
        method: 'POST', headers: { 'Content-Type': 'application/json' }, body: JSON.stringify(body),
        notifyOnError: true,
      })
      if (path === 'reauthenticate') toast.success('Identity verified', { description: 'Sensitive actions are available for the next 15 minutes.' })
      else if (path === 'sessions/revoke-others') toast.success('Other sessions revoked', { description: 'Your current session stays signed in.' })
      else if (path.startsWith('sessions/')) toast.success('Session revoked')
      else if (path === 'mfa/confirm') toast.success('Two-factor authentication enabled', { description: 'Save your recovery codes before closing this dialog.' })
      else if (path === 'mfa/disable') toast.success('Two-factor authentication disabled')
      else if (path === 'change-password') toast.success('Password changed', { description: 'Sign in again with your new password.' })
      try { await refresh() } catch {
        const notice = 'Action completed, but security settings could not be refreshed. Refresh the page to see the latest status.'
        setMessage(notice)
        toast.error(notice)
      }
      return result
    } catch (error) { setMessage(errorText(error)); return null }
    finally { setBusy(false) }
  }
  async function enroll(event: FormEvent<HTMLFormElement>) { const values = data(event); const result = await action('mfa/enroll', values); if (result) setEnrollment(result as { secret: string; otpauth_url: string }) }
  async function confirm(event: FormEvent<HTMLFormElement>) { const result = await action('mfa/confirm', data(event)); if (result) { setCodes((result as { recovery_codes: string[] }).recovery_codes); setEnrollment(null) } }
  return <div className="flex min-w-0 flex-col gap-6">
    <div className="space-y-1"><h1 className="text-2xl font-semibold">Security</h1><p className="text-sm text-muted-foreground">Protect your account, manage signed-in devices and review security activity.</p></div>
    <Message message={message} />
    <Tabs value={tab} onValueChange={setTab} className="min-w-0 gap-6">
      <div className="min-w-0 overflow-x-auto pb-1">
        <TabsList aria-label="Security sections" className="h-auto min-h-10 w-max sm:w-auto">
          <TabsTrigger value="verification" className="min-h-9 px-3">Verification</TabsTrigger>
          <TabsTrigger value="sessions" className="min-h-9 px-3">Sessions</TabsTrigger>
          <TabsTrigger value="two-factor" className="min-h-9 px-3">Two-factor</TabsTrigger>
          <TabsTrigger value="password" className="min-h-9 px-3">Password</TabsTrigger>
          <TabsTrigger value="activity" className="min-h-9 px-3">Activity</TabsTrigger>
        </TabsList>
      </div>
      <TabsContent value="verification" forceMount className={tabClass('verification')}>
    <section className={panel}><h2 className="font-semibold">Verify your identity</h2><p className="text-sm text-muted-foreground">Sensitive actions need a password verification within the last 15 minutes. Authenticator codes can be used only once per time step.</p><form className={formClass} onSubmit={async e => { await action('reauthenticate', data(e)) }}><FieldGroup><TextField name="password" label="Current password" type="password" autoComplete="current-password" /><TextField name="code" label="Authenticator or recovery code (if enabled)" required={false} autoComplete="one-time-code" /></FieldGroup><Button type="submit" className="self-start" disabled={busy}>Verify identity</Button></form></section>
      </TabsContent>
      <TabsContent value="sessions" forceMount className={tabClass('sessions')}>
    <section className={panel}><h2 className="font-semibold">Active sessions</h2><p className="text-sm text-muted-foreground">Sessions expire after three days of inactivity, with an absolute limit of 30 days.</p><p className="text-sm text-muted-foreground">Revoke other sessions signs out your other devices and keeps this session active. Verify your identity in the Verification tab within 15 minutes before using it.</p><DataTable label="Sessions" data={sessions} columns={[
      { id: 'device', label: 'Device', value: session => `${session.user_agent} ${session.ip_address} ${session.current ? 'Current' : ''}`, cell: session => <div className="flex flex-col gap-1"><p className="max-w-sm truncate" title={session.user_agent}>{session.user_agent || 'Unknown device'}</p><span className="text-xs text-muted-foreground">{session.ip_address}</span>{session.current ? <Badge variant="secondary" className="self-start">Current</Badge> : null}</div> },
      { id: 'expires', label: 'Expires', value: session => session.expires_at, cell: session => <time dateTime={session.expires_at}>{new Date(session.expires_at).toLocaleString()}</time> },
      { id: 'actions', label: 'Action', cell: session => session.current ? <Badge variant="outline" title="Use Sign out to end your current session">Protected</Badge> : <Button variant="outline" disabled={busy} onClick={() => action(`sessions/${session.id}/revoke`)}>Revoke</Button> },
    ]} /><Button variant="outline" className="self-start" disabled={busy} onClick={() => action('sessions/revoke-others')}>Revoke other sessions</Button></section>
      </TabsContent>
      <TabsContent value="two-factor" forceMount className={tabClass('two-factor')}>
    <section className={panel}><h2 ref={mfaHeading} tabIndex={-1} className="font-semibold">Two-factor authentication</h2><p className="text-sm text-muted-foreground">{security.two_factor_enabled ? `Enabled. ${security.recovery_codes_remaining} recovery codes remain.` : 'Add an authenticator app to protect your account.'}</p>
      {security.two_factor_enabled ? <form className={formClass} onSubmit={async e => { await action('mfa/disable', data(e)) }}><FieldGroup><TextField name="password" label="Confirm password" type="password" autoComplete="current-password" /><TextField name="code" label="Authenticator or recovery code" autoComplete="one-time-code" /></FieldGroup><Button type="submit" variant="outline" className="self-start" disabled={busy}>Disable two-factor authentication</Button></form> : null}
      <Dialog open={setupOpen} onOpenChange={open => {
        if (busy || codes.length) return
        setSetupOpen(open); setMessage('')
        if (!open) setEnrollment(null)
      }}>
        {!security.two_factor_enabled ? <DialogTrigger asChild><Button variant="outline" className="self-start" disabled={busy}>Set up authenticator</Button></DialogTrigger> : null}
        <DialogContent className="max-h-[calc(100svh-2rem)] overflow-y-auto" showCloseButton={!busy && !codes.length}
          onEscapeKeyDown={event => { if (busy || codes.length) event.preventDefault() }}
          onInteractOutside={event => { if (busy || codes.length) event.preventDefault() }}
          onCloseAutoFocus={event => { if (security.two_factor_enabled) { event.preventDefault(); mfaHeading.current?.focus() } }}>
          <DialogHeader>
            <DialogTitle>{codes.length ? 'Save your recovery codes' : enrollment ? 'Scan authenticator QR code' : 'Set up two-factor authentication'}</DialogTitle>
            <DialogDescription>{codes.length ? 'Keep these codes somewhere safe. Each code works once and will not be shown again.' : enrollment ? 'Scan with your authenticator app, then enter its 6-digit code. Setup expires in 10 minutes.' : 'Confirm your password to connect an authenticator app to your Ferrix account.'}</DialogDescription>
          </DialogHeader>
          <Message message={message} />
          {!enrollment && !codes.length ? <form onSubmit={enroll} className="flex flex-col gap-4"><FieldGroup><TextField name="password" label="Confirm password" type="password" autoComplete="current-password" /></FieldGroup><Button type="submit" disabled={busy}>{busy ? 'Please wait…' : 'Continue'}</Button></form> : null}
          {enrollment ? <div className="flex min-w-0 flex-col gap-4">
            <QRCodeSVG value={enrollment.otpauth_url} size={200} level="M" marginSize={4} title="Scan to set up Ferrix two-factor authentication" className="max-w-full self-center rounded-md" />
            <p className="text-sm text-muted-foreground">Cannot scan? Enter this setup key manually in your authenticator app.</p>
            <code className="break-all select-all rounded-md bg-muted p-3">{enrollment.secret}</code>
            <form onSubmit={confirm} className="flex flex-col gap-4"><FieldGroup><VerificationCode /></FieldGroup><Button type="submit" disabled={busy}>Enable two-factor authentication</Button></form>
          </div> : null}
          {codes.length ? <><pre className="min-w-0 select-all whitespace-pre-wrap break-all rounded-md bg-muted p-3 text-sm">{codes.join('\n')}</pre><Button onClick={() => { setCodes([]); setSetupOpen(false); setMessage('') }}>I saved my codes</Button></> : null}
        </DialogContent>
      </Dialog>
    </section>
      </TabsContent>
      <TabsContent value="password" forceMount className={tabClass('password')}>
    <section className={panel}><h2 className="font-semibold">Change password</h2><p className="text-sm text-muted-foreground">Use at least 12 characters. Changing your password signs out every session, including this one.</p><form onSubmit={async e => { await action('change-password', data(e)) }} className={formClass}><FieldGroup><TextField name="password" label="Existing password" type="password" autoComplete="current-password" /><TextField name="new_password" label="New password" type="password" autoComplete="new-password" /><TextField name="code" label="Authenticator or recovery code (if enabled)" required={false} autoComplete="one-time-code" /></FieldGroup><Button className="self-start" disabled={busy} type="submit">Change password and sign out</Button></form></section>
      </TabsContent>
      <TabsContent value="activity" forceMount className={tabClass('activity')}>
        <section className={panel}><h2 className="font-semibold">Organization security activity</h2><p className="text-sm text-muted-foreground">Recent security events for your current organization.</p>
          {audit.length ? <ul className="space-y-3">{audit.map(item => <li key={item.id} className="flex min-w-0 flex-col gap-1 border-b pb-3 last:border-0 last:pb-0"><span className="break-words text-sm">{item.action}</span><time className="text-xs text-muted-foreground" dateTime={item.created_at}>{new Date(item.created_at).toLocaleString()}</time></li>)}</ul> : <p className="text-sm text-muted-foreground">No security activity yet.</p>}
        </section>
      </TabsContent>
    </Tabs>
  </div>
}
