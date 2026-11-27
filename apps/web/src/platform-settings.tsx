import { useEffect, useState } from 'react'
import type { FormEvent } from 'react'
import { Link } from '@tanstack/react-router'
import { request } from '@/lib/api'
import { Alert, AlertTitle, AlertDescription } from '@/components/ui/alert'
import { Button } from '@/components/ui/button'
import { Card, CardHeader, CardTitle, CardDescription, CardContent } from '@/components/ui/card'
import { Field, FieldLabel, FieldDescription } from '@/components/ui/field'
import { Checkbox } from '@/components/ui/checkbox'
import { Input } from '@/components/ui/input'
import { NativeSelect, NativeSelectOption } from '@/components/ui/native-select'
import { Tabs, TabsList, TabsTrigger, TabsContent } from '@/components/ui/tabs'

type Section = 'timezone' | 'email' | 'social'
type Values = Record<string, string | number | boolean>
type Snapshot = Record<Section, Values & { revision: number }> & { timezones: string[] }
const fields: Record<'email' | 'social', [string, string, string?][]> = {
  email: [['host', 'SMTP host'], ['port', 'SMTP port', 'number'], ['username', 'SMTP username'], ['from_address', 'Sender email', 'email'], ['from_name', 'Sender name']],
  social: [['google_client_id', 'Google client ID'], ['github_client_id', 'GitHub client ID']],
}
const secrets: Record<'email' | 'social', [string, string][]> = {
  email: [['password', 'SMTP password'], ['resend_api_key', 'Resend API key']], social: [['google_client_secret', 'Google client secret'], ['github_client_secret', 'GitHub client secret']],
}
export function PlatformSettings({ admin, refreshVersion, refresh }: { admin: boolean; refreshVersion: number; refresh: () => Promise<void> }) {
  const [emailProvider, setEmailProvider] = useState('smtp')
  const [settings, setSettings] = useState<Snapshot | null>(null)
  const [error, setError] = useState(''), [message, setMessage] = useState(''), [busy, setBusy] = useState(false), [retry, setRetry] = useState(0)
  useEffect(() => {
    if (!admin) return
    const abort = new AbortController()
    setError('')
    request<Snapshot>('/auth/platform-settings', { signal: abort.signal }).then(value => { if (!abort.signal.aborted) { setSettings(value); setEmailProvider(String(value.email.provider ?? 'smtp')) } }).catch(reason => { if (!abort.signal.aborted) setError(reason.message) })
    return () => abort.abort()
  }, [admin, refreshVersion, retry])
  async function save(section: Section, event: FormEvent<HTMLFormElement>) {
    event.preventDefault()
    if (!settings) return
    const input = new FormData(event.currentTarget), values: Record<string, string | number> = {}
    for (const [key, value] of input.entries()) if (!key.startsWith('clear_')) values[key] = String(value)
    if (section !== 'timezone') for (const [key] of secrets[section]) {
      if (input.get(`clear_${key}`)) values[key] = ''
      else if (!values[key]) delete values[key]
    }
    if (section === 'email' && 'port' in values) values.port = Number(values.port)
    setBusy(true); setMessage(''); setError('')
    try {
      await request(`/auth/platform-settings/${section}`, { method: 'POST', headers: { 'Content-Type': 'application/json' }, body: JSON.stringify({ revision: settings[section].revision, values }) })
      setMessage('Settings saved.')
      await refresh()
    } catch (reason) { setError(reason instanceof Error ? reason.message : 'Could not save settings.') }
    finally { setBusy(false) }
  }
  if (!admin) return <Alert><AlertTitle>Platform administrator access required</AlertTitle><AlertDescription>Only platform administrators can manage installation settings.</AlertDescription></Alert>
  return <div className="flex min-w-0 flex-col gap-6">
    <div className="space-y-1"><h1 className="text-2xl font-semibold">Platform Settings</h1><p className="text-sm text-muted-foreground">Configuration shared by every organization on this Ferrix installation.</p></div>
    <p className="text-sm text-muted-foreground">Saving requires recent identity verification. <Link className="text-foreground underline underline-offset-4" to="/security">Verify your identity in Security</Link>.</p>
    {message ? <Alert role="status"><AlertTitle>{message}</AlertTitle></Alert> : null}
    {error ? <Alert variant="destructive"><AlertTitle>Settings need attention</AlertTitle><AlertDescription>{error}<Button variant="outline" disabled={busy} onClick={() => setRetry(value => value + 1)}>Reload settings</Button></AlertDescription></Alert> : null}
    {!settings ? <p role="status">Loading platform settings…</p> : <Tabs defaultValue="timezone" className="min-w-0 gap-6">
      <div className="overflow-x-auto pb-1"><TabsList aria-label="Platform settings sections" className="h-auto min-h-10"><TabsTrigger value="timezone" className="min-h-9 px-3">General</TabsTrigger><TabsTrigger value="email" className="min-h-9 px-3">Email</TabsTrigger><TabsTrigger value="social" className="min-h-9 px-3">Social Login</TabsTrigger></TabsList></div>
      <TabsContent value="timezone"><Card><CardHeader><CardTitle><h2>Platform timezone</h2></CardTitle><CardDescription>Controls the server clock shown throughout Ferrix. Scheduled backups retain their own timezone.</CardDescription></CardHeader><CardContent>
        <form key={settings.timezone.revision} className="flex max-w-xl flex-col gap-4" onSubmit={event => void save('timezone', event)}><fieldset disabled={busy}><Field><FieldLabel htmlFor="platform-timezone">Timezone</FieldLabel><NativeSelect id="platform-timezone" name="timezone" defaultValue={String(settings.timezone.timezone)}><NativeSelectOption value="">Server default</NativeSelectOption>{settings.timezones.map(zone => <NativeSelectOption key={zone} value={zone}>{zone}</NativeSelectOption>)}</NativeSelect><FieldDescription>Current effective timezone: {settings.timezone.effective_timezone}</FieldDescription></Field></fieldset><Button disabled={busy} className="self-start">Save timezone</Button></form>
      </CardContent></Card></TabsContent>
      {(['email', 'social'] as const).map(section => <TabsContent key={section} value={section}><Card><CardHeader><CardTitle><h2>{section === 'email' ? 'Email configuration' : 'Social login configuration'}</h2></CardTitle><CardDescription>{section === 'email' ? 'Choose Google App Password, Resend or SMTP for future email delivery. Invitations continue to use manually shared tokens; no email is sent yet.' : 'Save Google and GitHub credentials for future sign-in support. OAuth is not active yet.'}</CardDescription></CardHeader><CardContent>
        <form key={settings[section].revision} className="flex max-w-xl flex-col gap-6" onSubmit={event => void save(section, event)}><fieldset disabled={busy} className="flex flex-col gap-4">
          {section === 'email' ? <Field><FieldLabel htmlFor="platform-email-provider">Email provider</FieldLabel><NativeSelect id="platform-email-provider" name="provider" value={emailProvider} onChange={event => setEmailProvider(event.target.value)}><NativeSelectOption value="google">Google App Password</NativeSelectOption><NativeSelectOption value="resend">Resend API</NativeSelectOption><NativeSelectOption value="smtp">SMTP</NativeSelectOption></NativeSelect><FieldDescription>{emailProvider === 'google' ? 'Uses your Google sender email and app password with Gmail SMTP (STARTTLS).' : emailProvider === 'resend' ? 'Store a Resend API key and sender email verified in Resend.' : 'Use your own SMTP server.'}</FieldDescription></Field> : null}
          {fields[section].filter(([key]) => section !== 'email' || emailProvider === 'smtp' || ['from_address', 'from_name'].includes(key)).map(([key, label, type]) => <Field key={key}><FieldLabel htmlFor={`platform-${key}`}>{label}</FieldLabel><Input id={`platform-${key}`} name={key} type={type ?? 'text'} defaultValue={String(settings[section][key])} maxLength={section === 'social' ? 2048 : key === 'host' ? 253 : key === 'from_name' ? 120 : key === 'from_address' ? 254 : 320} min={type === 'number' ? 1 : undefined} max={type === 'number' ? 65535 : undefined} required={key === 'port'} autoComplete="off" /></Field>)}
          {section === 'email' && emailProvider === 'smtp' ? <Field><FieldLabel htmlFor="platform-security">SMTP security</FieldLabel><NativeSelect id="platform-security" name="security" defaultValue={String(settings.email.security)}><NativeSelectOption value="starttls">STARTTLS</NativeSelectOption><NativeSelectOption value="tls">TLS</NativeSelectOption></NativeSelect></Field> : null}
          {secrets[section].filter(([key]) => section !== 'email' || key === (emailProvider === 'resend' ? 'resend_api_key' : 'password')).map(([key, originalLabel]) => { const label = section === 'email' && emailProvider === 'google' ? 'Google app password' : originalLabel; return <Field key={key}><FieldLabel htmlFor={`platform-${key}`}>{label}</FieldLabel><Input id={`platform-${key}`} name={key} type="password" maxLength={4096} autoComplete="new-password" /><FieldDescription>{settings[section][`${key}_configured`] ? 'A secret is saved. Leave blank to keep it, or enter a replacement.' : 'No secret is saved. Enter a value to store it securely.'}</FieldDescription>{settings[section][`${key}_configured`] ? <label className="flex items-center gap-2 text-sm"><Checkbox name={`clear_${key}`} />Remove saved {label.toLowerCase()}</label> : null}</Field> })}
        </fieldset><Button disabled={busy} className="self-start">{busy ? 'Saving…' : section === 'email' ? 'Save email settings' : 'Save social login settings'}</Button></form>
      </CardContent></Card></TabsContent>)}
    </Tabs>}
  </div>
}
