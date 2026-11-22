import { useEffect, useId, useState } from 'react'
import { Link } from '@tanstack/react-router'
import type { Identity } from './auth'
import { request } from '@/lib/api'
import { Button } from '@/components/ui/button'
import { Input } from '@/components/ui/input'
import { Badge } from '@/components/ui/badge'
import { Checkbox } from '@/components/ui/checkbox'
import { NativeSelect, NativeSelectOption } from '@/components/ui/native-select'
import { Field, FieldLabel, FieldGroup, FieldSet, FieldLegend, FieldContent, FieldDescription } from '@/components/ui/field'
import { Alert, AlertTitle, AlertDescription } from '@/components/ui/alert'
import { Card, CardHeader, CardTitle, CardDescription, CardContent } from '@/components/ui/card'
import { Dialog, DialogTrigger, DialogContent, DialogHeader, DialogTitle, DialogDescription } from '@/components/ui/dialog'
import { DataTable } from '@/components/data-table'

type Key = { id: string; name: string; prefix: string; scopes: string[]; created_at: string; expires_at: string; revoked_at: string | null; last_used_at: string | null }
type Usage = { id: string; method: string; route: string; resource_id: string | null; status: number; ip_address: string; created_at: string }
const permissions = [
  ['read', 'Read projects and services', 'Lists projects, environments, deployment history, runtime and logs.'],
  ['manage', 'Manage projects and services', 'Creates projects/environments/services and updates configuration.'],
  ['deploy', 'Deploy and control services', 'Deploys images and starts, stops or reloads services.'],
  ['delete', 'Delete resources', 'Deletes services and empty projects. Enable only when needed.'],
  ['secrets:read', 'Read configuration secrets', 'Can retrieve environment variables and database credentials.'],
] as const
function Notice({ message }: { message: string }) { return message ? <Alert><AlertTitle>API key notice</AlertTitle><AlertDescription>{message}</AlertDescription></Alert> : null }
function state(key: Key) { return key.revoked_at ? 'Revoked' : Date.parse(key.expires_at) <= Date.now() ? 'Expired' : 'Active' }
function CreateKey({ identity, created }: { identity: Identity; created: () => void }) {
  const [open, setOpen] = useState(false); const [secret, setSecret] = useState(''); const [busy, setBusy] = useState(false); const [message, setMessage] = useState('')
  const initial = identity.role === 'member' ? ['read'] : ['read', 'manage', 'deploy']
  const [scopes, setScopes] = useState<string[]>(initial)
  const id = useId()
  return <Dialog open={open} onOpenChange={value => { if (busy || secret) return; setOpen(value); setMessage(''); if (value) setScopes(initial) }}>
    <DialogTrigger asChild><Button>Create API key</Button></DialogTrigger>
    <DialogContent className="max-h-[calc(100svh-2rem)] overflow-y-auto" showCloseButton={!busy && !secret} onEscapeKeyDown={event => { if (busy || secret) event.preventDefault() }} onInteractOutside={event => { if (busy || secret) event.preventDefault() }}>
      <DialogHeader><DialogTitle>{secret ? 'Save your API key' : 'Create API key'}</DialogTitle><DialogDescription>{secret ? 'This key will be shown once. Store it in your agent’s secret store; anyone holding it can use its permissions.' : `Create a key for ${identity.organization_name}. Permissions never exceed your current organization role. Up to 20 active keys per account.`}</DialogDescription></DialogHeader>
      <Notice message={message} />
      {secret ? <><code className="select-all break-all rounded-md bg-muted p-3" aria-label="New API key">{secret}</code><p className="text-sm text-muted-foreground">Use the Authorization: Bearer header. Never put this key in a URL, repository, prompt shared with others or logs.</p><Button onClick={() => { setSecret(''); setOpen(false) }}>I saved this key</Button></> : <form className="flex flex-col gap-4" onSubmit={async event => {
        event.preventDefault(); const form = new FormData(event.currentTarget); setBusy(true); setMessage('')
        try { const result = await request<{ key: string }>('/auth/api-keys', { method: 'POST', headers: { 'Content-Type': 'application/json' }, body: JSON.stringify({ name: form.get('name'), scopes, expires_in_days: Number(form.get('days')) }) }); setSecret(result.key); created() }
        catch (error) { setMessage(error instanceof Error ? error.message : 'Could not create key.') } finally { setBusy(false) }
      }}><fieldset disabled={busy}><FieldGroup>
        <Field><FieldLabel htmlFor={`${id}-name`}>Key name</FieldLabel><Input id={`${id}-name`} name="name" placeholder="e.g. My deployment agent" required maxLength={80} autoFocus /></Field>
        <Field><FieldLabel htmlFor={`${id}-days`}>Expires after</FieldLabel><NativeSelect id={`${id}-days`} name="days" defaultValue="90">{[1, 7, 30, 90, 180, 365].map(days => <NativeSelectOption key={days} value={String(days)}>{days} days</NativeSelectOption>)}</NativeSelect></Field>
        <FieldSet><FieldLegend>Permissions</FieldLegend><FieldGroup>{permissions.map(([scope, label, description]) => <Field key={scope} orientation="horizontal"><Checkbox id={`${id}-${scope}`} checked={scopes.includes(scope)} disabled={scope === 'read' || identity.role === 'member'} onCheckedChange={checked => setScopes(current => checked === true ? [...current, scope] : current.filter(value => value !== scope))} /><FieldContent><FieldLabel htmlFor={`${id}-${scope}`}>{label}</FieldLabel><FieldDescription>{description}</FieldDescription></FieldContent></Field>)}</FieldGroup></FieldSet>
        <FieldDescription>Manage/deploy can execute code with saved service secrets; grant them only to trusted agents. Members have read-only access. Creation requires identity verification within 15 minutes. <Link to="/security" className="underline underline-offset-4">Verify in Security</Link> when prompted.</FieldDescription>
      </FieldGroup></fieldset><Button type="submit" disabled={busy}>{busy ? 'Creating…' : 'Generate key'}</Button></form>}
    </DialogContent>
  </Dialog>
}
export function ApiKeysPage({ identity }: { identity: Identity }) {
  const [version, setVersion] = useState(0); const [keys, setKeys] = useState<Key[]>([]); const [loading, setLoading] = useState(true); const [message, setMessage] = useState(''); const [loadError, setLoadError] = useState('')
  const [revoking, setRevoking] = useState<Key | null>(null); const [busy, setBusy] = useState(false); const [selected, setSelected] = useState<Key | null>(null); const [usage, setUsage] = useState<Usage[] | null>(null); const [usageError, setUsageError] = useState('')
  const [guide, setGuide] = useState(false)
  useEffect(() => { const abort = new AbortController(); setLoading(true); setLoadError('')
    request<{ keys: Key[] }>('/auth/api-keys', { signal: abort.signal }).then(result => { if (!abort.signal.aborted) { setKeys(result.keys); setLoading(false) } }).catch(error => { if (!abort.signal.aborted) { setLoadError(error.message); setLoading(false) } })
    return () => abort.abort()
  }, [version])
  useEffect(() => { setUsage(null); setUsageError(''); if (!selected) return; const abort = new AbortController()
    request<{ requests: Usage[] }>(`/auth/api-keys/${selected.id}/usage`, { signal: abort.signal }).then(result => { if (!abort.signal.aborted) setUsage(result.requests) }).catch(error => { if (!abort.signal.aborted) setUsageError(error.message) })
    return () => abort.abort()
  }, [selected])
  return <div className="flex flex-col gap-6"><div className="flex flex-wrap items-center justify-between gap-4"><h1 className="text-2xl font-semibold">API keys</h1><CreateKey identity={identity} created={() => setVersion(value => value + 1)} /></div>
    <Notice message={message || loadError} />
    <Card><CardHeader><CardTitle>Agent access</CardTitle><CardDescription>Keys belong to your account and this organization: {identity.organization_name}. Create a separate key for each agent. Ban, password change/reset and membership removal invalidate keys. Keys cannot manage users, organizations or the Docker host.</CardDescription></CardHeader><CardContent className="flex flex-col gap-4"><p className="text-sm text-muted-foreground">120 requests per minute per key. Usage metadata is retained for 30 days; no request bodies or credentials are recorded. Lists show up to 100 keys, with active keys first.</p><div className="flex flex-wrap gap-2"><Button variant="outline" onClick={() => setGuide(true)}>Agent instructions</Button><Button variant="outline" onClick={() => setVersion(value => value + 1)}>Refresh API keys</Button></div></CardContent></Card>
    <Card><CardHeader><CardTitle>Your keys</CardTitle></CardHeader><CardContent><DataTable label="API keys" loading={loading} data={keys} columns={[
      { id: 'name', label: 'Key', value: key => key.name, cell: key => <div><p>{key.name}</p><code className="text-xs text-muted-foreground">{key.prefix}…</code></div> },
      { id: 'scopes', label: 'Permissions', value: key => key.scopes.join(' '), cell: key => <div className="flex max-w-xs flex-wrap gap-1">{key.scopes.map(scope => <Badge key={scope} variant="secondary">{scope}</Badge>)}</div> },
      { id: 'state', label: 'Status', value: state, cell: key => <Badge variant="outline">{state(key)}</Badge> },
      { id: 'expires', label: 'Expires', value: key => key.expires_at, cell: key => <time dateTime={key.expires_at}>{new Date(key.expires_at).toLocaleString()}</time> },
      { id: 'used', label: 'Last used', value: key => key.last_used_at ?? '', cell: key => key.last_used_at ? <time dateTime={key.last_used_at}>{new Date(key.last_used_at).toLocaleString()}</time> : 'Never' },
      { id: 'actions', label: 'Actions', cell: key => <div className="flex gap-2"><Button variant="outline" onClick={() => setSelected(key)}>Usage</Button><Button variant="outline" disabled={state(key) !== 'Active'} onClick={() => { setMessage(''); setRevoking(key) }}>Revoke</Button></div> },
    ]} /></CardContent></Card>
    <Dialog open={!!revoking} onOpenChange={value => { if (!value && !busy) setRevoking(null) }}><DialogContent showCloseButton={!busy} onEscapeKeyDown={event => { if (busy) event.preventDefault() }} onInteractOutside={event => { if (busy) event.preventDefault() }}><DialogHeader><DialogTitle>Revoke {revoking?.name}?</DialogTitle><DialogDescription>New requests using this key will be rejected immediately. An already-running deployment continues. This cannot be undone.</DialogDescription></DialogHeader><Notice message={message} /><div className="flex justify-end gap-2"><Button variant="outline" disabled={busy} onClick={() => setRevoking(null)}>Cancel</Button><Button variant="destructive" disabled={busy} onClick={async () => { if (!revoking) return; setBusy(true); setMessage(''); try { await request(`/auth/api-keys/${revoking.id}/revoke`, { method: 'POST' }); setRevoking(null); setVersion(value => value + 1) } catch (error) { setMessage(error instanceof Error ? error.message : 'Could not revoke key.') } finally { setBusy(false) } }}>Revoke key</Button></div></DialogContent></Dialog>
    <Dialog open={!!selected} onOpenChange={value => { if (!value) setSelected(null) }}><DialogContent className="max-h-[calc(100svh-2rem)] overflow-y-auto sm:max-w-3xl"><DialogHeader><DialogTitle>Usage: {selected?.name}</DialogTitle><DialogDescription>Latest 100 requests from the last 30 days. Routes contain templates, not URLs, credentials or request bodies.</DialogDescription></DialogHeader><Notice message={usageError} /><DataTable label="API key usage" loading={!usage && !usageError} data={usage ?? []} columns={[
      { id: 'time', label: 'Time', value: row => row.created_at, cell: row => new Date(row.created_at).toLocaleString() }, { id: 'request', label: 'Request', value: row => `${row.method} ${row.route}`, cell: row => <div><code>{row.method} {row.route}</code>{row.resource_id ? <p className="text-xs text-muted-foreground">{row.resource_id}</p> : null}</div> }, { id: 'status', label: 'Status', value: row => String(row.status), cell: row => <Badge variant={row.status >= 400 ? 'destructive' : 'outline'}>{row.status}</Badge> },
    ]} /></DialogContent></Dialog>
    <Dialog open={guide} onOpenChange={setGuide}><DialogContent className="max-h-[calc(100svh-2rem)] overflow-y-auto"><DialogHeader><DialogTitle>Instructions for your AI agent</DialogTitle><DialogDescription>Provide the API key separately through your agent’s secret store, not in the instructions.</DialogDescription></DialogHeader><pre className="whitespace-pre-wrap break-words rounded-md bg-muted p-3 text-sm">{`Use Ferrix at ${window.location.origin}/api/v1.\nRead the key from FERRIX_API_KEY.\nSend Authorization: Bearer <key> on each request. Do not send browser cookies or put credentials in URLs.\nStart with GET /agent to discover the organization, role and key scopes.\nList or create a project, find its environment, create an application/database, deploy it and poll deployment history/runtime until ready.\nNever delete resources or read configuration secrets without explicit authorization.\nOn 429, respect Retry-After. On 401, stop and request a valid key.\nRead GET /agent/docs for endpoint payloads and the deploy-image agent script.`}</pre></DialogContent></Dialog>
  </div>
}
