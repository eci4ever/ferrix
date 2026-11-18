import { useCallback, useEffect, useRef, useState } from 'react'
import type { FormEvent } from 'react'
import { Link } from '@tanstack/react-router'
import { ArrowLeft, ArrowUpRight, Box, Check, ChevronDown, CircleAlert, Copy, Database, Layers, Plus, Rocket, Search, Trash2 } from 'lucide-react'
import { ApplicationWorkspace, ConfigurationFields, configurationFrom } from './deployments'
import { request } from '@/lib/api'
import { Alert, AlertDescription, AlertTitle } from '@/components/ui/alert'
import { Badge } from '@/components/ui/badge'
import { Button } from '@/components/ui/button'
import { Card, CardAction, CardContent, CardDescription, CardHeader, CardTitle } from '@/components/ui/card'
import { DropdownMenu, DropdownMenuContent, DropdownMenuGroup, DropdownMenuItem, DropdownMenuLabel, DropdownMenuSeparator, DropdownMenuTrigger } from '@/components/ui/dropdown-menu'
import { Empty, EmptyDescription, EmptyHeader, EmptyMedia, EmptyTitle } from '@/components/ui/empty'
import { Field, FieldDescription, FieldGroup, FieldLabel } from '@/components/ui/field'
import { Input } from '@/components/ui/input'
import { InputGroup, InputGroupAddon, InputGroupInput } from '@/components/ui/input-group'
import { Skeleton } from '@/components/ui/skeleton'
import { Textarea } from '@/components/ui/textarea'
import { Collapsible, CollapsibleContent, CollapsibleTrigger } from '@/components/ui/collapsible'

type Kind = 'application' | 'database' | 'compose'
type Service = { id: string; environment_id: string; project_id: string; name: string; image: string; kind: Kind; deployed: boolean; created_at: string }
type Environment = { id: string; name: string }
type EnvironmentList = { project_name: string; environments: Environment[] }
type Settings = { name: string; database_name?: string; username?: string; password?: string; connection_uri?: string; credentials_locked?: boolean; yaml?: string; environment?: string }
type Runtime = { state: string; internal_host?: string; internal_port?: number; containers?: { id: string; name: string; state: string; ports: { port: number; ip: string; protocol: string }[] }[] }
type Deployment = { id: string; status: string; logs: string; created_at: string }
const types = { application: { label: 'Application', icon: Box }, database: { label: 'Database', icon: Database }, compose: { label: 'Compose', icon: Layers } }
const composeExample = 'services:\n  web:\n    image: nginx:alpine\n    ports:\n      - "80"\n'
const post = (value: unknown) => ({ method: 'POST', headers: { 'Content-Type': 'application/json' }, body: JSON.stringify(value) })
const message = (error: unknown) => error instanceof Error ? error.message : 'The request could not be completed.'
function ErrorNotice({ error }: { error: string }) { return error ? <Alert variant="destructive"><CircleAlert /><AlertTitle>Something needs attention</AlertTitle><AlertDescription>{error}</AlertDescription></Alert> : null }

function ServiceFields({ kind, settings, busy }: { kind: 'database' | 'compose'; settings?: Settings; busy: boolean }) {
  return <FieldGroup>
    <Field><FieldLabel htmlFor="service-name">Service name</FieldLabel><Input id="service-name" name="name" required maxLength={80} autoFocus disabled={busy} defaultValue={settings?.name ?? ''} placeholder={kind === 'database' ? 'e.g. PostgreSQL' : 'e.g. Web stack'} /></Field>
    {kind === 'database' ? <>
      <Field><FieldLabel htmlFor="database-name">Database name</FieldLabel><Input id="database-name" name="database_name" required maxLength={63} defaultValue={settings?.database_name ?? 'ferrix'} readOnly={settings?.credentials_locked} disabled={busy} /></Field>
      <Field><FieldLabel htmlFor="database-user">Username</FieldLabel><Input id="database-user" name="username" required maxLength={63} defaultValue={settings?.username ?? 'ferrix'} readOnly={settings?.credentials_locked} disabled={busy} /></Field>
      {settings ? <Field><FieldLabel htmlFor="database-password">Password</FieldLabel><Input id="database-password" name="password" type="password" required minLength={16} maxLength={128} defaultValue={settings.password} readOnly={settings.credentials_locked} disabled={busy} autoComplete="off" /><FieldDescription>{settings.credentials_locked ? 'Credentials are locked after deployment has been queued. Existing database data is preserved.' : 'You can change credentials before the first deployment.'}</FieldDescription></Field> : <FieldDescription>PostgreSQL 17. A strong password is generated automatically. Storage persists across redeploys, and the database has no public port.</FieldDescription>}
    </> : <>
      <Field><FieldLabel htmlFor="compose-yaml">Compose YAML</FieldLabel><Textarea id="compose-yaml" name="yaml" rows={14} required maxLength={65536} spellCheck={false} defaultValue={settings?.yaml ?? composeExample} disabled={busy} /><FieldDescription>Use public images and declared named volumes. Server file mounts, builds and external resources are not supported. Published ports use automatic server loopback ports.</FieldDescription></Field>
      <Field><FieldLabel htmlFor="compose-variables">Environment variables</FieldLabel><Textarea id="compose-variables" name="environment" rows={4} maxLength={16384} spellCheck={false} defaultValue={settings?.environment ?? ''} disabled={busy} placeholder="KEY=value" /><FieldDescription>Values used for Compose interpolation are stored encrypted.</FieldDescription></Field>
    </>}
  </FieldGroup>
}
function serviceForm(form: HTMLFormElement, kind: 'database' | 'compose') {
  const data = new FormData(form)
  return kind === 'database' ? { name: String(data.get('name')), database_name: String(data.get('database_name')), username: String(data.get('username')), ...(data.has('password') ? { password: String(data.get('password')) } : {}) } : { name: String(data.get('name')), yaml: String(data.get('yaml')), environment: String(data.get('environment') ?? '') }
}

export function ProjectServices({ projectId, canManage, canCompose }: { projectId: string; canManage: boolean; canCompose: boolean }) {
  const [data, setData] = useState<EnvironmentList | null>(null)
  const [selected, setSelected] = useState('')
  const [services, setServices] = useState<Service[] | null>(null)
  const [form, setForm] = useState<Kind | 'environment' | null>(null)
  const [busy, setBusy] = useState(false)
  const [error, setError] = useState('')
  const [notice, setNotice] = useState('')
  const [filter, setFilter] = useState('')
  const [type, setType] = useState<Kind | ''>('')
  const [deleting, setDeleting] = useState<Service | null>(null)
  const [confirmation, setConfirmation] = useState('')
  const [removeData, setRemoveData] = useState(false)
  const [sort, setSort] = useState<'recent' | 'name'>('recent')
  const createButton = useRef<HTMLButtonElement>(null)
  const environmentButton = useRef<HTMLButtonElement>(null)
  const choosing = useRef(false)
  const refresh = useCallback(async (signal?: AbortSignal) => {
    const next = await request<EnvironmentList>(`/projects/${projectId}/environments`, { signal })
    setData(next)
    setSelected(current => next.environments.some(e => e.id === current) ? current : (next.environments.find(e => e.name === 'production')?.id ?? next.environments[0]?.id ?? ''))
  }, [projectId])
  useEffect(() => { const controller = new AbortController(); void refresh(controller.signal).catch(e => { if (!controller.signal.aborted) setError(message(e)) }); return () => controller.abort() }, [refresh])
  useEffect(() => {
    if (!selected) return
    const controller = new AbortController()
    setServices(null); setError(''); setFilter(''); setType(''); setDeleting(null)
    void request<Service[]>(`/environments/${selected}/services`, { signal: controller.signal }).then(next => { if (!controller.signal.aborted) setServices(next) }).catch(e => { if (!controller.signal.aborted) setError(message(e)) })
    return () => controller.abort()
  }, [selected])
  function close() { const previous = form; setForm(null); setError(''); (previous === 'environment' ? environmentButton : createButton).current?.focus() }
  async function create(event: FormEvent<HTMLFormElement>) {
    event.preventDefault(); if (!form) return
    const element = event.currentTarget
    const body = form === 'environment' ? { name: String(new FormData(element).get('name')) } : form === 'application' ? { kind: form, ...configurationFrom(element) } : { kind: form, ...serviceForm(element, form) }
    setBusy(true); setError(''); setNotice('')
    try {
      if (form === 'environment') { const environment = await request<Environment>(`/projects/${projectId}/environments`, post(body)); await refresh(); setSelected(environment.id); close(); setNotice(`Environment “${environment.name}” created.`) }
      else { const service = await request<Service>(`/environments/${selected}/services`, post(body)); setServices(current => [service, ...(current ?? [])]); close(); setNotice(`${types[service.kind].label} “${service.name}” created. Open it to configure and deploy.`) }
    } catch (e) { setError(message(e)) } finally { setBusy(false) }
  }
  async function deleteService(event: FormEvent<HTMLFormElement>) {
    event.preventDefault(); if (!deleting || confirmation !== deleting.name) return
    setBusy(true); setError(''); setNotice('')
    try {
      await request(`/services/${deleting.id}`, { method: 'DELETE', headers: { 'Content-Type': 'application/json' }, body: JSON.stringify({ name: confirmation, remove_data: removeData }), signal: AbortSignal.timeout(100_000) })
      setServices(current => current?.filter(service => service.id !== deleting.id) ?? null)
      setNotice(`Service “${deleting.name}” deleted.${removeData ? ' Stored data was removed.' : deleting.kind !== 'application' ? ' Stored data was retained in Docker volumes.' : ''}`)
      setDeleting(null); setConfirmation(''); setRemoveData(false)
    } catch (e) { setError(message(e)) } finally { setBusy(false) }
  }
  const active = data?.environments.find(e => e.id === selected)
  const visible = services?.filter(s => (!type || s.kind === type) && `${s.name} ${s.image}`.toLowerCase().includes(filter.toLowerCase())).sort((a, b) => sort === 'name' ? a.name.localeCompare(b.name) : new Date(b.created_at).getTime() - new Date(a.created_at).getTime())
  return <>
    <Button asChild variant="ghost" className="self-start"><Link to="/projects"><ArrowLeft data-icon="inline-start" />Projects</Link></Button>
    <div className="flex flex-wrap items-center justify-between gap-4">
      <div className="flex flex-col gap-1"><div className="flex flex-wrap items-center gap-2"><h1 className="text-2xl font-semibold tracking-tight">{data?.project_name ?? 'Project'}</h1><span className="text-muted-foreground">/</span><DropdownMenu><DropdownMenuTrigger asChild><Button variant="ghost" disabled={busy || !!deleting || !data?.environments.length}>{active?.name ?? 'Environment'}<ChevronDown data-icon="inline-end" /></Button></DropdownMenuTrigger><DropdownMenuContent><DropdownMenuGroup>{data?.environments.map(environment => <DropdownMenuItem key={environment.id} onSelect={() => { setSelected(environment.id); setForm(null); setNotice('') }}>{environment.name}{environment.id === selected ? ' (selected)' : ''}</DropdownMenuItem>)}</DropdownMenuGroup></DropdownMenuContent></DropdownMenu></div><p className="text-sm text-muted-foreground">Services in the {active?.name ?? 'selected'} environment.</p></div>
      {canManage ? <div className="flex flex-wrap gap-2"><Button ref={environmentButton} variant="outline" onClick={() => { setForm('environment'); setNotice('') }} disabled={busy}>Project Environment</Button><DropdownMenu><DropdownMenuTrigger asChild><Button ref={createButton} disabled={busy || !selected}><Plus data-icon="inline-start" />Create Service</Button></DropdownMenuTrigger><DropdownMenuContent align="end" onCloseAutoFocus={event => { if (choosing.current) { event.preventDefault(); choosing.current = false } }}><DropdownMenuGroup><DropdownMenuLabel>Actions</DropdownMenuLabel><DropdownMenuSeparator />{(Object.keys(types) as Kind[]).map(kind => { const Icon = types[kind].icon; return <DropdownMenuItem key={kind} disabled={kind === 'compose' && !canCompose} onSelect={() => { choosing.current = true; setForm(kind); setError(''); setNotice('') }}><Icon />{types[kind].label}</DropdownMenuItem> })}</DropdownMenuGroup></DropdownMenuContent></DropdownMenu></div> : null}
    </div>
    <p role="status" className="text-sm empty:hidden">{notice}</p><ErrorNotice error={error} />
    {deleting ? <Card><CardHeader><CardTitle><h2>Delete {deleting.name}?</h2></CardTitle><CardDescription>This stops and removes the service containers, configuration and deployment history. This action cannot be undone.</CardDescription></CardHeader><CardContent><form onSubmit={deleteService} className="flex flex-col gap-4"><FieldGroup>{deleting.kind !== 'application' ? <Field><FieldLabel>Stored data</FieldLabel><DropdownMenu><DropdownMenuTrigger asChild><Button variant="outline" disabled={busy}>{removeData ? 'Permanently delete data' : 'Keep data volumes'}<ChevronDown data-icon="inline-end" /></Button></DropdownMenuTrigger><DropdownMenuContent><DropdownMenuGroup><DropdownMenuItem onSelect={() => setRemoveData(false)}>Keep data volumes</DropdownMenuItem><DropdownMenuItem onSelect={() => setRemoveData(true)}>Permanently delete data</DropdownMenuItem></DropdownMenuGroup></DropdownMenuContent></DropdownMenu><FieldDescription>{removeData ? 'Database and Compose volumes owned by this service will be permanently deleted. Back up any data you need first.' : 'Docker volumes remain on the server for manual recovery. They are not automatically attached to a new service.'}</FieldDescription></Field> : null}<Field><FieldLabel htmlFor="delete-service-name">Type the service name to confirm</FieldLabel><Input id="delete-service-name" value={confirmation} onChange={event => setConfirmation(event.target.value)} autoComplete="off" autoFocus disabled={busy} required /><FieldDescription>Enter {deleting.name} exactly.</FieldDescription></Field></FieldGroup><div className="flex flex-wrap justify-end gap-2"><Button type="button" variant="outline" disabled={busy} onClick={() => { setDeleting(null); setError('') }}>Cancel</Button><Button variant="destructive" disabled={busy || confirmation !== deleting.name}>{busy ? 'Deleting…' : 'Delete service'}</Button></div></form></CardContent></Card> : null}
    {form ? <Card><CardHeader><CardTitle><h2>{form === 'environment' ? 'Create environment' : `Create ${types[form].label}`}</h2></CardTitle><CardDescription>{form === 'environment' ? 'Separate production and staging services.' : `Add a service to ${active?.name}.`}</CardDescription></CardHeader><CardContent><form onSubmit={create} className="flex flex-col gap-4">
      {form === 'environment' ? <FieldGroup><Field><FieldLabel htmlFor="environment-name">Environment name</FieldLabel><Input id="environment-name" name="name" required maxLength={80} autoFocus disabled={busy} placeholder="staging" /></Field></FieldGroup> : form === 'application' ? <ConfigurationFields disabled={busy} /> : <ServiceFields key={form} kind={form} busy={busy} />}
      <div className="flex justify-end gap-2"><Button type="button" variant="outline" disabled={busy} onClick={close}>Cancel</Button><Button disabled={busy}>{busy ? 'Creating…' : form === 'environment' ? 'Create environment' : `Create ${types[form].label}`}</Button></div>
    </form></CardContent></Card> : null}
    <div className="flex flex-wrap items-center gap-3"><InputGroup className="min-w-0 basis-full sm:flex-1 sm:basis-0"><InputGroupAddon><Search /></InputGroupAddon><InputGroupInput aria-label="Filter services" placeholder="Filter services…" value={filter} onChange={event => setFilter(event.target.value)} /></InputGroup><DropdownMenu><DropdownMenuTrigger asChild><Button variant="outline">{sort === 'recent' ? 'Recently created' : 'Name (A–Z)'}<ChevronDown data-icon="inline-end" /></Button></DropdownMenuTrigger><DropdownMenuContent><DropdownMenuGroup><DropdownMenuItem onSelect={() => setSort('recent')}>Recently created</DropdownMenuItem><DropdownMenuItem onSelect={() => setSort('name')}>Name (A–Z)</DropdownMenuItem></DropdownMenuGroup></DropdownMenuContent></DropdownMenu><DropdownMenu><DropdownMenuTrigger asChild><Button variant="outline">{type ? types[type].label : 'All types'}<ChevronDown data-icon="inline-end" /></Button></DropdownMenuTrigger><DropdownMenuContent><DropdownMenuGroup><DropdownMenuItem onSelect={() => setType('')}>All types</DropdownMenuItem>{(Object.keys(types) as Kind[]).map(kind => <DropdownMenuItem key={kind} onSelect={() => setType(kind)}>{types[kind].label}{type === kind ? <Check /> : null}</DropdownMenuItem>)}</DropdownMenuGroup></DropdownMenuContent></DropdownMenu></div>
    {!services && !error ? <Skeleton className="h-48 w-full" /> : visible?.length ? <div className="grid gap-4 md:grid-cols-2 xl:grid-cols-3">{visible.map(service => { const Icon = types[service.kind].icon; return <Card key={service.id}><CardHeader><CardTitle><h2><Link to="/projects/$projectId/services/$serviceId" params={{ projectId, serviceId: service.id }}>{service.name}</Link></h2></CardTitle><CardDescription>{service.kind === 'database' ? 'PostgreSQL · persistent storage' : service.kind === 'compose' ? 'Docker Compose' : service.image}</CardDescription><CardAction>{canManage && (service.kind !== 'compose' || canCompose) ? <Button variant="ghost" size="icon" aria-label={`Delete ${service.name}`} disabled={busy} onClick={() => { setDeleting(service); setConfirmation(''); setRemoveData(false); setForm(null); setError(''); setNotice('') }}><Trash2 /></Button> : <Icon />}</CardAction></CardHeader><CardContent className="flex flex-col gap-4"><div className="flex flex-wrap gap-2"><Badge variant="outline">{types[service.kind].label}</Badge><Badge variant={service.deployed ? 'secondary' : 'outline'}>{service.deployed ? 'Deployed' : 'Not deployed'}</Badge></div><div className="flex items-center justify-between gap-2"><time className="text-xs text-muted-foreground" dateTime={service.created_at}>{new Date(service.created_at).toLocaleDateString()}</time><Button asChild variant="secondary"><Link to="/projects/$projectId/services/$serviceId" params={{ projectId, serviceId: service.id }}>Open<ArrowUpRight data-icon="inline-end" /></Link></Button></div></CardContent></Card> })}</div> : !error ? <Card><CardContent><Empty><EmptyHeader><EmptyMedia variant="icon"><Layers /></EmptyMedia><EmptyTitle>{services?.length ? 'No matching services' : `No services in ${active?.name ?? 'this environment'}`}</EmptyTitle><EmptyDescription>{services?.length ? 'Try a different search or service type.' : 'Create an Application, Database or Compose service to get started.'}</EmptyDescription></EmptyHeader></Empty></CardContent></Card> : null}
  </>
}

export function ServiceWorkspace({ projectId, serviceId, canManage, canCompose }: { projectId: string; serviceId: string; canManage: boolean; canCompose: boolean }) {
  const [service, setService] = useState<Service | null>(null)
  const [error, setError] = useState('')
  useEffect(() => { const controller = new AbortController(); void request<Service>(`/services/${serviceId}`, { signal: controller.signal }).then(row => { if (row.project_id !== projectId) throw new Error('Service not found in this project.'); if (!controller.signal.aborted) setService(row) }).catch(e => { if (!controller.signal.aborted) setError(message(e)) }); return () => controller.abort() }, [projectId, serviceId])
  if (error) return <ErrorNotice error={error} />
  if (!service) return <Skeleton className="h-48 w-full" />
  if (service.kind === 'application') return <ApplicationWorkspace projectId={projectId} applicationId={serviceId} canManage={canManage} />
  return <ManagedService key={serviceId} initial={service} projectId={projectId} canManage={canManage && (service.kind !== 'compose' || canCompose)} />
}
function ManagedService({ initial, projectId, canManage }: { initial: Service; projectId: string; canManage: boolean }) {
  const [service, setService] = useState(initial)
  const [runtime, setRuntime] = useState<Runtime | null>(null)
  const [deployments, setDeployments] = useState<Deployment[]>([])
  const [settings, setSettings] = useState<Settings | null>(null)
  const [editing, setEditing] = useState(false)
  const [connection, setConnection] = useState(false)
  const [busy, setBusy] = useState(false)
  const [error, setError] = useState('')
  const [pollError, setPollError] = useState('')
  const [notice, setNotice] = useState('')
  const [logs, setLogs] = useState<string | null>(null)
  const inFlight = useRef(false)
  const configurationButton = useRef<HTMLButtonElement>(null)
  const refresh = useCallback(async (signal?: AbortSignal) => {
    if (inFlight.current) return
    inFlight.current = true
    try { const [row, running, history] = await Promise.all([request<Service>(`/services/${initial.id}`, { signal }), request<Runtime>(`/services/${initial.id}/runtime`, { signal }), request<Deployment[]>(`/services/${initial.id}/deployments`, { signal })]); if (!signal?.aborted) { setService(row); setRuntime(running); setDeployments(history); setPollError('') } }
    finally { inFlight.current = false }
  }, [initial.id])
  useEffect(() => { const controller = new AbortController(); const poll = () => { if (!document.hidden) void refresh(controller.signal).catch(e => { if (!controller.signal.aborted) setPollError(message(e)) }) }; poll(); const timer = window.setInterval(poll, 3000); document.addEventListener('visibilitychange', poll); return () => { controller.abort(); window.clearInterval(timer); document.removeEventListener('visibilitychange', poll) } }, [refresh])
  async function action(operation: () => Promise<void>) { setBusy(true); setError(''); setNotice(''); try { await operation() } catch (e) { setError(message(e)) } finally { setBusy(false) } }
  function configure() { void action(async () => { setSettings(await request<Settings>(`/services/${service.id}/configuration`)); setEditing(true) }) }
  async function save(event: FormEvent<HTMLFormElement>) { event.preventDefault(); const value = serviceForm(event.currentTarget, service.kind as 'database' | 'compose'); await action(async () => { setService(await request<Service>(`/services/${service.id}/configuration`, post(value))); setEditing(false); setSettings(null); setConnection(false); setNotice('Configuration saved. Deploy to apply it.'); configurationButton.current?.focus() }) }
  const pending = deployments.some(d => d.status === 'queued' || d.status === 'running')
  return <>
    <Button asChild variant="ghost" className="self-start"><Link to="/projects/$projectId" params={{ projectId }}><ArrowLeft data-icon="inline-start" />Back to project</Link></Button>
    <div className="flex flex-wrap items-center justify-between gap-4"><div><h1 className="text-2xl font-semibold tracking-tight">{service.name}</h1><p className="text-sm text-muted-foreground">{service.kind === 'database' ? 'PostgreSQL 17 · persistent storage' : 'Docker Compose'}</p></div><div className="flex flex-wrap gap-2"><Button variant="outline" disabled={busy || runtime?.state === 'not_deployed'} onClick={() => void action(async () => { const response = await request<{ logs: string }>(`/services/${service.id}/logs`); setLogs(response.logs || 'No output yet.') })}>Container logs</Button>{canManage ? <><Button ref={configurationButton} variant="outline" onClick={configure} disabled={busy}>Configuration</Button><Button disabled={busy || editing || pending} onClick={() => void action(async () => { await request(`/services/${service.id}/deploy`, post({})); setSettings(null); setConnection(false); setNotice('Deployment queued.'); await refresh() })}><Rocket data-icon="inline-start" />{pending ? 'Deploying…' : service.deployed ? 'Redeploy' : 'Deploy'}</Button></> : null}</div></div>
    <p role="status" className="text-sm empty:hidden">{notice}</p><ErrorNotice error={error || pollError} />
    <Card><CardHeader><CardTitle><h2>Runtime</h2></CardTitle><CardDescription>{service.kind === 'database' ? 'Private database on the environment network. No host port is published.' : 'Containers managed by this Compose service.'}</CardDescription><CardAction><Badge variant={runtime?.state === 'running' ? 'secondary' : 'outline'}>{runtime?.state ?? 'Checking…'}</Badge></CardAction></CardHeader><CardContent className="flex flex-col gap-3">{service.kind === 'database' ? <><p className="break-all text-sm">Internal host: {runtime?.internal_host ?? `ferrix-${service.id.replaceAll('-', '')}`}:5432</p>{canManage ? <Button variant="outline" className="self-start" disabled={busy} onClick={() => void action(async () => { setSettings(await request<Settings>(`/services/${service.id}/configuration`)); setConnection(true) })}>Connection details</Button> : null}{connection && settings ? <FieldGroup><Field><FieldLabel htmlFor="database-uri">Connection URL</FieldLabel><div className="flex gap-2"><Input id="database-uri" type="password" readOnly value={settings.connection_uri ?? ''} /><Button variant="outline" aria-label="Copy connection URL" onClick={() => void action(async () => { await navigator.clipboard.writeText(settings.connection_uri ?? ''); setNotice('Connection URL copied.') })}><Copy /></Button></div><FieldDescription>Use this URL in an application deployed in the same environment. Redeploy existing applications to join its network.</FieldDescription></Field></FieldGroup> : null}</> : runtime?.containers?.length ? runtime.containers.map(container => <div key={container.id} className="flex flex-wrap items-center gap-3"><span>{container.name}</span><Badge variant="outline">{container.state}</Badge>{container.ports?.map(port => port.protocol === 'tcp' ? <Button key={port.port} asChild variant="outline"><a href={`http://127.0.0.1:${port.port}`} target="_blank" rel="noopener noreferrer">127.0.0.1:{port.port}<ArrowUpRight data-icon="inline-end" /></a></Button> : <span key={port.port}>{port.port}/udp</span>)}</div>) : <p className="text-sm text-muted-foreground">Deploy to start the services defined in your YAML.</p>}</CardContent></Card>
    {editing && settings ? <Card><CardHeader><CardTitle><h2>Service configuration</h2></CardTitle><CardDescription>{service.kind === 'database' ? 'Database data is retained across redeploys. A redeploy briefly restarts the database.' : 'Compose updates this project in place. A failed update can leave some services running; check runtime and logs before retrying.'}</CardDescription></CardHeader><CardContent><form onSubmit={save} className="flex flex-col gap-4"><ServiceFields kind={service.kind as 'database' | 'compose'} settings={settings} busy={busy} /><div className="flex justify-end gap-2"><Button type="button" variant="outline" disabled={busy} onClick={() => { setEditing(false); setSettings(null); setConnection(false); configurationButton.current?.focus() }}>Cancel</Button><Button disabled={busy}>{busy ? 'Saving…' : 'Save configuration'}</Button></div></form></CardContent></Card> : null}
    {logs !== null ? <Card><CardHeader><CardTitle><h2>Container logs</h2></CardTitle><CardAction><Button variant="ghost" onClick={() => setLogs(null)}>Close</Button></CardAction></CardHeader><CardContent><pre className="max-h-80 overflow-auto whitespace-pre-wrap break-all text-xs">{logs}</pre></CardContent></Card> : null}
    <Card><CardHeader><CardTitle><h2>Deployments</h2></CardTitle><CardDescription>Latest 30 deployments. Progress refreshes automatically.</CardDescription></CardHeader><CardContent className="flex flex-col gap-4">{deployments.length ? deployments.map(deployment => <Collapsible key={deployment.id} defaultOpen={deployment.status === 'failed' || deployment.status === 'running'}><div className="flex flex-wrap items-center gap-3"><Badge variant={deployment.status === 'failed' ? 'destructive' : deployment.status === 'succeeded' ? 'secondary' : 'outline'}>{deployment.status}</Badge><time dateTime={deployment.created_at} className="text-xs text-muted-foreground">{new Date(deployment.created_at).toLocaleString()}</time><CollapsibleTrigger asChild><Button variant="ghost">Progress</Button></CollapsibleTrigger></div><CollapsibleContent><pre className="max-h-64 overflow-auto whitespace-pre-wrap break-all text-xs">{deployment.logs || 'Waiting for worker…'}</pre></CollapsibleContent></Collapsible>) : <Empty><EmptyHeader><EmptyMedia variant="icon"><Rocket /></EmptyMedia><EmptyTitle>Ready to deploy</EmptyTitle><EmptyDescription>Save your configuration, then deploy this service.</EmptyDescription></EmptyHeader></Empty>}</CardContent></Card>
  </>
}
