import { useEffect, useState } from 'react'
import { Link } from '@tanstack/react-router'
import { toast } from 'sonner'
import { request } from '@/lib/api'
import { Button } from '@/components/ui/button'
import { Dialog, DialogContent, DialogHeader, DialogTitle, DialogDescription } from '@/components/ui/dialog'
import { Card, CardContent, CardHeader, CardTitle, CardDescription } from '@/components/ui/card'
import { Field, FieldGroup, FieldLabel, FieldDescription } from '@/components/ui/field'
import { Input } from '@/components/ui/input'
import { NativeSelect, NativeSelectOption } from '@/components/ui/native-select'
import { Alert, AlertTitle, AlertDescription } from '@/components/ui/alert'
import { ApplicationSource } from './application-source'

type Record = { application_id: string; status: string; message: string; public_urls: string[]; lan_urls: string[]; diagnostic_url: string | null }
type Catalog = { runtime: string; document_root: string; dockerfile: string; files: { path: string; content: string }[] }
type Member = { id: string; name: string; email: string; role: string }
type Database = { id: string; name: string; database_engine?: string }
const post = (body: unknown = {}) => ({ method: 'POST', headers: { 'Content-Type': 'application/json' }, body: JSON.stringify(body), signal: AbortSignal.timeout(120_000) })

export function PhpTemplate({ environmentId, projectId, onCreated }: { environmentId: string; projectId: string; onCreated: (app: string) => Promise<void> }) {
  const [open, setOpen] = useState(false)
  const [catalog, setCatalog] = useState<Catalog | null>(null)
  const [members, setMembers] = useState<Member[]>([])
  const [databases, setDatabases] = useState<Database[]>([])
  const [requestId, setRequestId] = useState('')
  const [record, setRecord] = useState<Record | null>(null)
  const [busy, setBusy] = useState(false)
  const [error, setError] = useState('')
  const [name, setName] = useState('')
  const [student, setStudent] = useState('')
  const [database, setDatabase] = useState('')
  const [memory, setMemory] = useState('256')
  const [cpu, setCpu] = useState('0.5')
  const [created, setCreated] = useState(false)
  async function action(work: () => Promise<void>) { setBusy(true); setError(''); try { await work() } catch (err) { setError(err instanceof Error ? err.message : 'PHP template operation failed.') } finally { setBusy(false) } }
  async function start() {
    setOpen(true); setRecord(null); setCreated(false); setRequestId(crypto.randomUUID())
    await action(async () => {
      const [template, users, services] = await Promise.all([request<Catalog>('/templates/php-v1'), request<Member[]>('/auth/members'), request<Database[]>(`/environments/${environmentId}/services`)])
      setCatalog(template); setMembers(users.filter(user => user.role === 'member')); setDatabases(services.filter(service => service.database_engine === 'mariadb'))
    })
  }
  return <><Button variant="outline" disabled={!environmentId} onClick={() => void start()}>Add student PHP application</Button><Dialog open={open} onOpenChange={value => { if (!busy) setOpen(value) }}><DialogContent className="max-h-[90svh] overflow-y-auto sm:max-w-3xl"><DialogHeader><DialogTitle>PHP + shared MariaDB</DialogTitle><DialogDescription>Create an assigned student application with a private SQL account. Use assignment-only access in Team and remove broad student project grants first.</DialogDescription></DialogHeader>
    {error && <Alert variant="destructive"><AlertTitle>Could not complete this step</AlertTitle><AlertDescription>{error} Retry this step; existing application and database data are retained.</AlertDescription></Alert>}
    {!record ? <form className="grid gap-4" onSubmit={event => { event.preventDefault(); void action(async () => { const next = await request<Record>(`/environments/${environmentId}/php-applications`, post({ request_id: requestId, name, student_id: student, database_service_id: database, memory_mib: Number(memory), cpus: Number(cpu) })); setRecord(next); await onCreated(next.application_id); toast.success('Student application created') }) }}><fieldset disabled={busy}><FieldGroup>
      <Field><FieldLabel htmlFor="php-name">Application name</FieldLabel><Input id="php-name" value={name} onChange={event => setName(event.target.value)} required maxLength={80} /></Field>
      <Field><FieldLabel htmlFor="php-student">Student</FieldLabel><NativeSelect id="php-student" value={student} onChange={event => setStudent(event.target.value)} required><NativeSelectOption value="">Choose organization member</NativeSelectOption>{members.map(user => <NativeSelectOption key={user.id} value={user.id}>{user.name} · {user.email}</NativeSelectOption>)}</NativeSelect><FieldDescription>The student receives a developer assignment for this application only.</FieldDescription></Field>
      <Field><FieldLabel htmlFor="php-database">Shared MariaDB service</FieldLabel><NativeSelect id="php-database" value={database} onChange={event => setDatabase(event.target.value)} required><NativeSelectOption value="">Choose MariaDB in this environment</NativeSelectOption>{databases.map(db => <NativeSelectOption key={db.id} value={db.id}>{db.name}</NativeSelectOption>)}</NativeSelect><FieldDescription>Deploy MariaDB first. Each application receives a separate logical database/account with 10 connections.</FieldDescription></Field>
      <div className="grid gap-4 sm:grid-cols-2"><Field><FieldLabel htmlFor="php-memory">Memory limit (MiB)</FieldLabel><Input id="php-memory" type="number" min={6} step={1} value={memory} onChange={event => setMemory(event.target.value)} required /></Field><Field><FieldLabel htmlFor="php-cpu">CPU limit (cores)</FieldLabel><Input id="php-cpu" type="number" min={0.01} step={0.01} value={cpu} onChange={event => setCpu(event.target.value)} required /></Field></div>
      <FieldDescription>{catalog?.runtime ?? 'PHP 8.5 / Apache'} · port 80 · document root public/ · persistent uploads/sessions in /opt/data. Existing organization quotas apply; disk capacity is advisory.</FieldDescription>
      <details><summary className="cursor-pointer text-sm font-medium">Preview generated files and build conventions</summary><p className="my-3 text-sm text-muted-foreground">The template uses {catalog?.dockerfile}. A user Dockerfile is retained. public/index.php is required. PHP must read DB_* from getenv(); legacy code is not rewritten. Composer dependencies require composer.lock and install during build without scripts. No CDN is required.</p>{catalog?.files.map(file => <details key={file.path} className="my-2"><summary className="cursor-pointer font-mono text-xs">{file.path}</summary><pre className="max-h-60 overflow-auto whitespace-pre-wrap rounded-md bg-muted p-3 text-xs">{file.content}</pre></details>)}<a className="text-sm underline" href="/api/v1/templates/php-v1/download" download>Download starter source</a></details>
    </FieldGroup></fieldset><Button disabled={busy || !student || !database || !catalog}>Create assigned application</Button></form> : <div className="grid gap-4">
      <Alert><AlertTitle>{record.status === 'ready' ? 'Database ready' : 'Review source, then provision database'}</AlertTitle><AlertDescription>{record.message}</AlertDescription></Alert>
      <ApplicationSource serviceId={record.application_id} canManage onSaved={async () => {}} />
      <p className="text-sm text-muted-foreground">Keep the starter, or import Git/ZIP through the existing Source controls. Choose the PHP template build explicitly to retain your Dockerfile. Only public/ is served. Provisioning saves DB_HOST, DB_PORT, DB_DATABASE, DB_USERNAME and DB_PASSWORD on the server; it preserves unrelated environment values.</p>
      <Button variant="outline" disabled={busy} onClick={() => void action(async () => { setRecord(await request<Record>(`/services/${record.application_id}/php-template/provision`, post())); toast.success('Database provisioned and connection settings saved') })}>Provision database and connection settings</Button>
      <Button disabled={busy || record.status !== 'ready' || created} onClick={() => void action(async () => { await request(`/services/${record.application_id}/deploy`, post()); setCreated(true); toast.success('PHP application deployment queued') })}>Deploy PHP application</Button>
      <Button variant="outline" asChild><Link to="/projects/$projectId/services/$serviceId" params={{ projectId, serviceId: record.application_id }}>Open application and deployment progress</Link></Button>
      <p className="text-sm text-muted-foreground">Public and LAN URLs are managed in the application's Domains tab. LAN ingress needs school DNS/hosts and the internal CA trusted on each device. Generating a hostname does not configure your router or certificates.</p>
    </div>}
  </DialogContent></Dialog></>
}

export function PhpTemplateStatus({ serviceId, canManage }: { serviceId: string; canManage: boolean }) {
  const [record, setRecord] = useState<Record | null>(null)
  const [error, setError] = useState('')
  useEffect(() => { const abort = new AbortController(); setRecord(null); request<Record | null>(`/services/${serviceId}/php-template`, { signal: abort.signal }).then(next => { if (!abort.signal.aborted) setRecord(next) }).catch(() => {}); return () => abort.abort() }, [serviceId])
  if (!record) return null
  return <Card><CardHeader><CardTitle><h2>PHP classroom template</h2></CardTitle><CardDescription>PHP v1 · public/ document root · shared MariaDB · persistent storage in /opt/data.</CardDescription></CardHeader><CardContent className="grid gap-3"><Button variant="outline" className="justify-self-start" onClick={async () => { try { setRecord(await request<Record | null>(`/services/${serviceId}/php-template`)); setError('') } catch (err) { setError(err instanceof Error ? err.message : 'Cannot load template status.') } }}>Check template status</Button>{error && <p role="alert">{error}</p>}{record && <><p role="status">{record.status} · {record.message}</p><div className="grid gap-3 text-sm"><div><p className="font-medium">Public URL</p>{record.public_urls.length ? record.public_urls.map(url => <a key={url} className="block break-all underline" href={url} target="_blank" rel="noreferrer">{url}</a>) : <p className="text-muted-foreground">Not configured. Ask your lecturer to configure a public direct/tunnel mapping in Domains.</p>}</div><div><p className="font-medium">LAN URL</p>{record.lan_urls.length ? record.lan_urls.map(url => <a key={url} className="block break-all underline" href={url} target="_blank" rel="noreferrer">{url}</a>) : <p className="text-muted-foreground">Not configured. LAN ingress requires school DNS/hosts, reachable ingress ports and internal CA trust.</p>}</div>{record.diagnostic_url && <a className="underline" href={record.diagnostic_url} target="_blank" rel="noreferrer">Open server-local diagnostic URL (not a LAN hostname)</a>}</div>{canManage && <Button variant="outline" className="justify-self-start" onClick={async () => { try { setRecord(await request<Record>(`/services/${serviceId}/php-template/provision`, post())); toast.success('Database provisioned and connection settings saved'); setError('') } catch (err) { setError(err instanceof Error ? err.message : 'Database provisioning failed.') } }}>Retry database provisioning</Button>}</>}</CardContent></Card>
}
