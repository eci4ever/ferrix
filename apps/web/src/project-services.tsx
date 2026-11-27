import { WorkspaceSkeleton } from '@/components/workspace-skeleton'
import { LoadingButton } from '@/components/loading-button'
import { PhpTemplate, PhpTemplateStatus } from './php-template'
import { ManagedDatabases, ApplicationDatabase } from './managed-databases'
import { ResourceAccess } from './resource-access'
import { useCallback, useEffect, useRef, useState } from 'react'
import type { FormEvent } from 'react'
import { Link } from '@tanstack/react-router'
import { ArrowLeft, ArrowUpRight, Globe, Check, ChevronDown, CircleAlert, Copy, Database, Layers, Plus, Rocket, Search, Trash2, RefreshCw, Play, Square, Eye, EyeOff } from 'lucide-react'
import { FormDialog } from '@/components/form-dialog'
import { ConfigurationFields, configurationFrom } from './deployments'
import { ProjectBreadcrumb } from './project-breadcrumb'
import { ProjectBackups } from './project-backups'
import { ResourceQuotas } from './resource-quotas'
import { ServiceTerminal } from './service-terminal'
import { ServiceLogs } from './service-logs'
import { ServiceMetrics } from './service-metrics'
import { ServiceNotes } from './service-notes'
import { ServiceTransfer } from './service-transfer'
import { ServiceBackups } from './backups'
import { ApplicationSource } from './application-source'
import { ServiceDomains } from './domains'
import { ServiceStatus, serviceState, statusLabel } from './service-status'
import { Tabs, TabsList, TabsTrigger, TabsContent } from '@/components/ui/tabs'
import { Dialog, DialogContent, DialogHeader, DialogTitle, DialogDescription, DialogFooter } from '@/components/ui/dialog'
import { RadioGroup, RadioGroupItem } from '@/components/ui/radio-group'
import { DataTable } from '@/components/data-table'
import { ActionNotice } from '@/components/action-notice'
import { request } from '@/lib/api'
import { Alert, AlertDescription, AlertTitle } from '@/components/ui/alert'
import { Badge } from '@/components/ui/badge'
import { Button } from '@/components/ui/button'
import { Card, CardAction, CardContent, CardDescription, CardHeader, CardTitle } from '@/components/ui/card'
import { DropdownMenu, DropdownMenuContent, DropdownMenuGroup, DropdownMenuItem, DropdownMenuLabel, DropdownMenuSeparator, DropdownMenuTrigger } from '@/components/ui/dropdown-menu'
import { Empty, EmptyDescription, EmptyHeader, EmptyMedia, EmptyTitle } from '@/components/ui/empty'
import { Field, FieldDescription, FieldGroup, FieldLabel, FieldSet, FieldLegend } from '@/components/ui/field'
import { Input } from '@/components/ui/input'
import { InputGroup, InputGroupAddon, InputGroupInput, InputGroupButton } from '@/components/ui/input-group'
import { Skeleton } from '@/components/ui/skeleton'
import { Textarea } from '@/components/ui/textarea'

type DatabaseEngine = 'postgres' | 'mongodb' | 'mariadb' | 'redis'
const databaseTypes = { postgres: { label: 'PostgreSQL', version: '17', port: 5432, logo: "/database-logos/postgres.svg" }, mongodb: { label: 'MongoDB', version: '8', port: 27017, logo: "/database-logos/mongodb.svg" }, mariadb: { label: 'MariaDB', version: '11.4', port: 3306, logo: "/database-logos/mariadb.svg" }, redis: { label: 'Redis', version: '8', port: 6379, logo: "/database-logos/redis.svg" } }
function DatabaseLogo({ engine, className = 'size-8' }: { engine: DatabaseEngine; className?: string }) {
  return <img src={databaseTypes[engine].logo} alt="" aria-hidden="true" className={className} width={32} height={32} />
}
type DockerKind = 'application' | 'database'
type Kind = DockerKind
type Service = { cutover_pending?: boolean; traffic_enabled?:boolean; restore_blocked?: boolean; worker_mode?: boolean; source_type?: string; source_repository?: string | null; database_operation_pending?: boolean; active_deployment_id?: string | null; active_image?: string | null; pending_changes?: boolean; automatic_url?: string | null; database_engine?: DatabaseEngine; id: string; environment_id: string; project_id: string; name: string; image: string; kind: Kind; deployed: boolean; created_at: string }
type DockerService = Omit<Service, 'kind'> & { kind: DockerKind }
type Environment = { id: string; name: string }
type EnvironmentList = { project_name: string; environments: Environment[] }
type Settings = { memory_mib?: number; cpus?: number; worker_mode?: boolean; command?: string[]; storage?: {target:string;read_only:boolean}|null; storage_volume?: string|null; engine?: DatabaseEngine; root_password?: string | null; name: string; image?: string; container_port?: number; database_name?: string; username?: string; password?: string; connection_uri?: string; credentials_locked?: boolean; environment?: string }
type Runtime = { state: string; health?: string; host_port?: number; container_id?: string; internal_host?: string; internal_port?: number }
type Deployment = { source_archive_sha256?: string | null; source_repository?: string | null; source_branch?: string | null; git_commit?: string | null; trigger_kind?: string; trigger_sender?: string | null; requested_by?: string | null; requested_by_name?: string | null; requested_by_key_id?: string | null; requested_by_key_name?: string | null; rollback_of?: string | null; image: string; image_id?: string | null; id: string; status: string; logs: string; created_at: string }
const types = { application: { label: 'Application', icon: Globe }, database: { label: 'Database', icon: Database } }
const post = (value: unknown) => ({ method: 'POST', headers: { 'Content-Type': 'application/json' }, body: JSON.stringify(value) })
const message = (error: unknown) => error instanceof Error ? error.message : 'The request could not be completed.'
function ErrorNotice({ error }: { error: string }) { return error ? <Alert variant="destructive"><CircleAlert /><AlertTitle>Something needs attention</AlertTitle><AlertDescription>{error}</AlertDescription></Alert> : null }

function ServiceFields({ settings, busy, engine = settings?.engine ?? 'postgres' }: { settings?: Settings; busy: boolean; engine?: DatabaseEngine }) {
  return <FieldGroup>
    <Field><FieldLabel htmlFor="service-name">Service name</FieldLabel><Input id="service-name" name="name" required maxLength={80} autoFocus disabled={busy} defaultValue={settings?.name ?? ''} placeholder={`e.g. ${databaseTypes[engine].label}`} /></Field>
    <>
      <input type="hidden" name="engine" value={engine} />
      {engine !== 'redis' ? <>
      <Field><FieldLabel htmlFor="database-name">Database name</FieldLabel><Input id="database-name" name="database_name" required maxLength={63} defaultValue={settings?.database_name ?? 'ferrix'} readOnly={settings?.credentials_locked} disabled={busy} /></Field>
      <Field><FieldLabel htmlFor="database-user">Username</FieldLabel><Input id="database-user" name="username" required maxLength={engine === 'mariadb' ? 32 : 63} defaultValue={settings?.username ?? 'ferrix'} readOnly={settings?.credentials_locked} disabled={busy} /></Field>
      </> : <><input type="hidden" name="database_name" value="0" /><input type="hidden" name="username" value="default" /></>}
      {settings ? <Field><FieldLabel htmlFor="database-password">Password</FieldLabel><Input id="database-password" name="password" type="password" required minLength={16} maxLength={128} defaultValue={settings.password} readOnly={settings.credentials_locked} disabled={busy} autoComplete="off" /><FieldDescription>{settings.credentials_locked ? 'Credentials are locked after deployment has been queued. Existing database data is preserved.' : 'You can change credentials before the first deployment.'}</FieldDescription></Field> : <FieldDescription>{databaseTypes[engine].label} {databaseTypes[engine].version}. A strong password is generated automatically. Storage persists across redeploys, and the database has no public port.</FieldDescription>}
      {!settings && engine === 'mongodb' ? <FieldDescription>MongoDB generates an administrative user in authSource=admin. The selected database is created on its first write.</FieldDescription> : null}
      {!settings && engine === 'mariadb' ? <FieldDescription>MariaDB generates separate application and root passwords.</FieldDescription> : null}
      {!settings && engine === 'redis' ? <FieldDescription>Redis uses database 0 with password authentication and AOF persistence.</FieldDescription> : null}
      {settings?.root_password ? <Field><FieldLabel htmlFor="database-root-password">Root password</FieldLabel><Input id="database-root-password" name="root_password" type="password" required minLength={16} maxLength={128} defaultValue={settings.root_password} readOnly={settings.credentials_locked} disabled={busy} autoComplete="off" /></Field> : null}
    </>
  </FieldGroup>
}
function serviceForm(form: HTMLFormElement) {
  const data = new FormData(form)
  return { engine: String(data.get('engine') ?? 'postgres'), root_password: data.has('root_password') ? String(data.get('root_password')) : null, name: String(data.get('name')), database_name: String(data.get('database_name')), username: String(data.get('username')), ...(data.has('password') ? { password: String(data.get('password')) } : {}) }
}

export function ProjectServices({ projectId, canManage, canAdmin=false }: { projectId: string; canManage: boolean; canAdmin?:boolean }) {
  const [data, setData] = useState<EnvironmentList | null>(null)
  const [selected, setSelected] = useState('')
  const [statuses, setStatuses] = useState<Record<string, string>>({})
  const [checkedAt, setCheckedAt] = useState<Date | null>(null)
  const [statusError, setStatusError] = useState(false)
  const [services, setServices] = useState<Service[] | null>(null)
  const [databaseEngine, setDatabaseEngine] = useState<DatabaseEngine>('postgres')
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
  const creationOrigin = useRef<'environment' | 'service'>('service')
  const deleteOrigin = useRef<HTMLButtonElement | null>(null)
  const createButton = useRef<HTMLButtonElement>(null)
  const environmentButton = useRef<HTMLButtonElement>(null)
  const choosing = useRef(false)
  const refresh = useCallback(async (signal?: AbortSignal) => {
    const next = await request<EnvironmentList>(`/projects/${projectId}/environments`, { signal })
    setData(next)
    setSelected(current => { const requested = new URLSearchParams(window.location.search).get('environment'); if (!current && next.environments.some(e => e.id === requested)) return requested!; return next.environments.some(e => e.id === current) ? current : (next.environments.find(e => e.name === 'production')?.id ?? next.environments[0]?.id ?? '') })
  }, [projectId])
  useEffect(() => { const controller = new AbortController(); void refresh(controller.signal).catch(e => { if (!controller.signal.aborted) setError(message(e)) }); return () => controller.abort() }, [refresh])
  useEffect(() => {
    if (!selected) return
    const controller = new AbortController()
    setServices(null); setError(''); setFilter(''); setType(''); setDeleting(null)
    void request<Service[]>(`/environments/${selected}/services`, { signal: controller.signal }).then(next => {
      if (!controller.signal.aborted) setServices(next)
    }).catch(e => { if (!controller.signal.aborted) setError(message(e)) })
    return () => controller.abort()
  }, [selected])
  useEffect(() => {
    if (!selected) return
    const controller = new AbortController(); let inFlight = false
    setStatuses({}); setCheckedAt(null); setStatusError(false)
    const poll = async () => {
      if (document.hidden || inFlight) return
      inFlight = true
      try { const rows = await request<{ id: string; state: string }[]>(`/environments/${selected}/service-status`, { signal: controller.signal }); if (!controller.signal.aborted) { setStatuses(Object.fromEntries(rows.map(row => [row.id, row.state]))); setCheckedAt(new Date()); setStatusError(false) } }
      catch { if (!controller.signal.aborted) { setStatuses({}); setStatusError(true) } }
      finally { inFlight = false }
    }
    void poll(); const timer = window.setInterval(() => void poll(), 5000)
    const onVisible = () => void poll(); document.addEventListener('visibilitychange', onVisible)
    return () => { controller.abort(); window.clearInterval(timer); document.removeEventListener('visibilitychange', onVisible) }
  }, [selected])
  function close() { setForm(null); setError('') }
  async function create(event: FormEvent<HTMLFormElement>) {
    event.preventDefault(); if (!form) return
    const element = event.currentTarget
    setBusy(true); setError(''); setNotice('')
    try {
    const body = form === 'environment' ? { name: String(new FormData(element).get('name')) } : form === 'application' ? { kind: form, ...configurationFrom(element) } : { kind: form, ...serviceForm(element) }
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
      setNotice(`Service “${deleting.name}” deleted.${removeData ? ' Stored data was removed.' : ' Any data volumes were retained.'}`)
      setDeleting(null); setConfirmation(''); setRemoveData(false)
    } catch (e) { setError(message(e)) } finally { setBusy(false) }
  }
  const active = data?.environments.find(e => e.id === selected)
  const visible = services?.filter(s => (!type || s.kind === type) && `${s.name} ${s.image}`.toLowerCase().includes(filter.toLowerCase())).sort((a, b) => sort === 'name' ? a.name.localeCompare(b.name) : new Date(b.created_at).getTime() - new Date(a.created_at).getTime())
  return <>
    <ProjectBreadcrumb projectId={projectId} projectName={data?.project_name} environmentId={selected} environmentName={active?.name} />
    <div className="flex flex-wrap items-center justify-between gap-4">
      <div className="flex flex-col gap-1"><div className="flex flex-wrap items-center gap-2"><h1 className="text-2xl font-semibold tracking-tight">{data?.project_name ?? 'Project'}</h1><span className="text-muted-foreground">/</span><DropdownMenu><DropdownMenuTrigger asChild><Button variant="ghost" disabled={busy || !!deleting || !data?.environments.length}>{active?.name ?? 'Environment'}<ChevronDown data-icon="inline-end" /></Button></DropdownMenuTrigger><DropdownMenuContent><DropdownMenuGroup>{data?.environments.map(environment => <DropdownMenuItem key={environment.id} onSelect={() => { setSelected(environment.id); setForm(null); setNotice('') }}>{environment.name}{environment.id === selected ? ' (selected)' : ''}</DropdownMenuItem>)}</DropdownMenuGroup></DropdownMenuContent></DropdownMenu></div><p className="text-sm text-muted-foreground">Services in the {active?.name ?? 'selected'} environment.</p></div>
      {canManage ? <div className="flex flex-wrap gap-2"><PhpTemplate environmentId={selected} projectId={projectId} onCreated={async id => { const service = await request<Service>(`/services/${id}`); setServices(current => [service, ...(current ?? []).filter(item => item.id !== service.id)]) }} /><Button ref={environmentButton} variant="outline" aria-haspopup="dialog" onClick={() => { creationOrigin.current = 'environment'; setForm('environment'); setNotice(''); setError('') }} disabled={busy}>Project Environment</Button><DropdownMenu><DropdownMenuTrigger asChild><Button ref={createButton} disabled={busy || !selected}><Plus data-icon="inline-start" />Create Service</Button></DropdownMenuTrigger><DropdownMenuContent align="end" onCloseAutoFocus={event => { if (choosing.current) { event.preventDefault(); choosing.current = false } }}><DropdownMenuGroup><DropdownMenuLabel>Actions</DropdownMenuLabel><DropdownMenuSeparator />{(Object.keys(types) as Kind[]).map(kind => { const Icon = types[kind].icon; return <DropdownMenuItem key={kind} onSelect={() => { choosing.current = true; creationOrigin.current = 'service'; setDatabaseEngine('postgres'); setForm(kind); setError(''); setNotice('') }}><Icon />{types[kind].label}</DropdownMenuItem> })}</DropdownMenuGroup></DropdownMenuContent></DropdownMenu></div> : null}
    </div>
    <ActionNotice message={notice} onDismiss={() => setNotice('')} />{!form && !deleting ? <ErrorNotice error={error} /> : null}
    <FormDialog open={deleting !== null} onOpenChange={open => { if (!open) { setDeleting(null); setError('') } }} title={`Delete ${deleting?.name ?? 'service'}?`} description="This stops and removes the service containers, configuration and deployment history. Managed application databases and SQL accounts are retained separately. Shared MariaDB services with managed databases cannot be deleted. This action cannot be undone." busy={busy}
      onCloseAutoFocus={event => { event.preventDefault(); (deleteOrigin.current?.isConnected ? deleteOrigin.current : createButton.current)?.focus() }}>
      <ErrorNotice error={error} />
      {deleting ? <form onSubmit={deleteService} className="flex flex-col gap-4"><FieldGroup><Field><FieldLabel>Stored data</FieldLabel><DropdownMenu><DropdownMenuTrigger asChild><Button variant="outline" disabled={busy}>{removeData ? 'Permanently delete data' : 'Keep data volumes'}<ChevronDown data-icon="inline-end" /></Button></DropdownMenuTrigger><DropdownMenuContent><DropdownMenuGroup><DropdownMenuItem onSelect={() => setRemoveData(false)}>Keep data volumes</DropdownMenuItem><DropdownMenuItem onSelect={() => setRemoveData(true)}>Permanently delete data</DropdownMenuItem></DropdownMenuGroup></DropdownMenuContent></DropdownMenu><FieldDescription>{removeData ? 'Data volumes owned by this service will be permanently deleted. Back up any data you need first.' : 'Docker volumes remain on the server for manual recovery. They are not automatically attached to a new service.'}</FieldDescription></Field><Field><FieldLabel htmlFor="delete-service-name">Type the service name to confirm</FieldLabel><Input id="delete-service-name" value={confirmation} onChange={event => setConfirmation(event.target.value)} autoComplete="off" autoFocus disabled={busy} required /><FieldDescription>Enter {deleting.name} exactly.</FieldDescription></Field></FieldGroup><div className="flex flex-wrap justify-end gap-2"><Button type="button" variant="outline" disabled={busy} onClick={() => { setDeleting(null); setError('') }}>Cancel</Button><Button variant="destructive" disabled={busy || confirmation !== deleting.name}>{busy ? 'Deleting…' : 'Delete service'}</Button></div></form> : null}
    </FormDialog>
    <FormDialog open={form !== null && form !== 'database'} onOpenChange={open => { if (!open) close() }} title={form === 'environment' ? 'Create environment' : 'Create Application'} description={form === 'environment' ? 'Separate production and staging services.' : `Add an application to ${active?.name ?? 'this environment'}. Save it first, then deploy when ready.`} busy={busy} className={form === 'application' ? 'sm:max-w-2xl' : undefined}
      onCloseAutoFocus={event => { event.preventDefault(); (creationOrigin.current === 'environment' ? environmentButton : createButton).current?.focus() }}>
      <ErrorNotice error={error} />
      {form && form !== 'database' ? <form onSubmit={create} className="flex flex-col gap-4">
        {form === 'environment' ? <FieldGroup><Field><FieldLabel htmlFor="environment-name">Environment name</FieldLabel><Input id="environment-name" name="name" required maxLength={80} autoFocus disabled={busy} placeholder="staging" /></Field></FieldGroup> : <ConfigurationFields disabled={busy} />}
        <DialogFooter><Button type="button" variant="outline" disabled={busy} onClick={close}>Cancel</Button><Button disabled={busy}>{busy ? 'Creating…' : form === 'environment' ? 'Create environment' : 'Create Application'}</Button></DialogFooter>
      </form> : null}
    </FormDialog>
    <Dialog open={form === 'database'} onOpenChange={open => { if (!open && !busy) close() }}><DialogContent className="max-h-[90dvh] overflow-y-auto sm:max-w-xl" showCloseButton={!busy} onEscapeKeyDown={event => { if (busy) event.preventDefault() }} onInteractOutside={event => { if (busy) event.preventDefault() }} onCloseAutoFocus={event => { event.preventDefault(); createButton.current?.focus() }}><DialogHeader><DialogTitle>Databases</DialogTitle><DialogDescription>Select a database to add to {active?.name ?? 'this environment'}.</DialogDescription></DialogHeader><ErrorNotice error={error} /><form onSubmit={create} className="flex flex-col gap-6"><FieldSet><FieldLegend>Select a database</FieldLegend><RadioGroup aria-label="Database type" value={databaseEngine} onValueChange={value => setDatabaseEngine(value as DatabaseEngine)} disabled={busy} className="grid grid-cols-2 gap-3">{(Object.keys(databaseTypes) as DatabaseEngine[]).map(engine => { const entry = databaseTypes[engine]; return <FieldLabel key={engine} htmlFor={`database-engine-${engine}`}><Field orientation="vertical" className="items-center gap-3"><DatabaseLogo engine={engine} className="size-8 shrink-0 object-contain" /><div className="flex items-center gap-2"><RadioGroupItem id={`database-engine-${engine}`} value={engine} /><span>{entry.label}</span></div></Field></FieldLabel> })}</RadioGroup></FieldSet><ServiceFields busy={busy} engine={databaseEngine} /><DialogFooter><Button type="button" variant="outline" disabled={busy} onClick={close}>Cancel</Button><Button disabled={busy}>{busy ? 'Creating…' : 'Create Database'}</Button></DialogFooter></form></DialogContent></Dialog>
    <div className="flex flex-wrap items-center gap-3"><InputGroup className="min-w-0 basis-full sm:flex-1 sm:basis-0"><InputGroupAddon><Search /></InputGroupAddon><InputGroupInput aria-label="Filter services" placeholder="Filter services…" value={filter} onChange={event => setFilter(event.target.value)} /></InputGroup><DropdownMenu><DropdownMenuTrigger asChild><Button variant="outline">{sort === 'recent' ? 'Recently created' : 'Name (A–Z)'}<ChevronDown data-icon="inline-end" /></Button></DropdownMenuTrigger><DropdownMenuContent><DropdownMenuGroup><DropdownMenuItem onSelect={() => setSort('recent')}>Recently created</DropdownMenuItem><DropdownMenuItem onSelect={() => setSort('name')}>Name (A–Z)</DropdownMenuItem></DropdownMenuGroup></DropdownMenuContent></DropdownMenu><DropdownMenu><DropdownMenuTrigger asChild><Button variant="outline">{type ? types[type].label : 'All types'}<ChevronDown data-icon="inline-end" /></Button></DropdownMenuTrigger><DropdownMenuContent><DropdownMenuGroup><DropdownMenuItem onSelect={() => setType('')}>All types</DropdownMenuItem>{(Object.keys(types) as Kind[]).map(kind => <DropdownMenuItem key={kind} onSelect={() => setType(kind)}>{types[kind].label}{type === kind ? <Check /> : null}</DropdownMenuItem>)}</DropdownMenuGroup></DropdownMenuContent></DropdownMenu></div>
    <p className="text-xs text-muted-foreground" role="status">{statusError ? 'Runtime status unavailable. Retrying automatically…' : checkedAt ? `Status checked ${checkedAt.toLocaleTimeString()} · refreshes every 5 seconds` : 'Checking runtime status…'}</p>
    {!services && !error ? <div className="grid gap-4 md:grid-cols-2 xl:grid-cols-3" role="status" aria-label="Loading services">{[0, 1, 2].map(item => <Card key={item}><CardHeader><Skeleton className="h-5 w-2/3" /><Skeleton className="mt-2 h-4 w-4/5" /></CardHeader><CardContent><Skeleton className="h-6 w-32" /></CardContent></Card>)}</div> : visible?.length ? <div className="grid gap-4 md:grid-cols-2 xl:grid-cols-3">{visible.map(service => { const Icon = types[service.kind].icon; return <Card key={service.id} className="relative min-w-0 transition-colors duration-150 hover:bg-muted/20 motion-reduce:transition-none"><CardHeader className="min-w-0"><CardTitle className="min-w-0 break-words"><h2><Link className="after:absolute after:inset-0 after:rounded-xl focus-visible:outline-none focus-visible:after:ring-2 focus-visible:after:ring-ring" to={"/projects/$projectId/services/$serviceId"} params={{ projectId, serviceId: service.id }}>{service.name}</Link></h2></CardTitle><CardDescription className="break-words leading-relaxed">{service.kind === 'database' ? `${databaseTypes[service.database_engine ?? 'postgres'].label} · persistent storage` : service.source_type === 'github' ? `GitHub · ${service.source_repository}` : service.source_type === 'upload' ? 'Uploaded source · Dockerfile' : service.image}</CardDescription><CardAction><div className="flex items-center gap-2">{service.kind === 'database' ? <DatabaseLogo engine={service.database_engine ?? 'postgres'} className="size-8 shrink-0 object-contain" /> : <Icon aria-hidden="true" className="size-8 shrink-0 text-muted-foreground" />}{canManage ? <Button variant="ghost" size="icon" className="relative" aria-label={`Delete ${service.name}`} disabled={busy} onClick={event => { deleteOrigin.current = event.currentTarget; setDeleting(service); setConfirmation(''); setRemoveData(false); setForm(null); setError(''); setNotice('') }}><Trash2 /></Button> : null}</div></CardAction></CardHeader><CardContent className="mt-auto flex flex-col gap-4"><div className="flex flex-wrap items-center gap-2"><Badge variant="outline">{types[service.kind].label}</Badge><Badge variant="outline" className="gap-2"><ServiceStatus state={statuses[service.id] ?? (statusError ? 'unavailable' : 'checking')} /><span aria-hidden="true" className="capitalize">{statusLabel(statuses[service.id] ?? (statusError ? 'unavailable' : 'checking'))}</span></Badge></div><div className="flex items-center justify-between gap-2"><time className="text-xs text-muted-foreground" dateTime={service.created_at}>{new Date(service.created_at).toLocaleDateString()}</time></div></CardContent></Card> })}</div> : !error ? <Card><CardContent><Empty><EmptyHeader><EmptyMedia variant="icon"><Layers /></EmptyMedia><EmptyTitle>{services?.length ? 'No matching services' : `No services in ${active?.name ?? 'this environment'}`}</EmptyTitle><EmptyDescription>{services?.length ? 'Try a different search or service type.' : 'Create a service to get started.'}</EmptyDescription></EmptyHeader>{services?.length && (filter || type) ? <Button variant="outline" onClick={() => { setFilter(''); setType('') }}>Clear filters</Button> : null}</Empty></CardContent></Card> : null}
    {canManage ? <ResourceQuotas projectId={projectId} canAdmin={canAdmin}/> : null}
    <ProjectBackups projectId={projectId} canManage={canManage}/>
    <ResourceAccess resource={`/projects/${projectId}`} canManage={canManage} />
  </>
}

export function ServiceWorkspace({ projectId, serviceId, canManage }: { projectId: string; serviceId: string; canManage: boolean }) {
  const [service, setService] = useState<DockerService | null>(null)
  const [project, setProject] = useState<EnvironmentList | null>(null)
  useEffect(() => { const controller = new AbortController(); void request<EnvironmentList>(`/projects/${projectId}/environments`, { signal: controller.signal }).then(setProject).catch(() => {}); return () => controller.abort() }, [projectId])
  const [access, setAccess] = useState<{permission:string;can_develop:boolean}|null>(null)
  const [error, setError] = useState('')
  useEffect(() => {
    if (canManage) return
    const controller = new AbortController()
    const poll = () => request<{permission:string;can_develop:boolean}>(`/services/${serviceId}/access`, {signal:controller.signal}).then(value => { if (!controller.signal.aborted) setAccess(value) }).catch(reason => { if (!controller.signal.aborted) { setAccess(null); setError(message(reason)) } })
    void poll(); const timer=setInterval(() => void poll(),5000)
    return () => { controller.abort(); clearInterval(timer) }
  }, [serviceId, canManage])
  useEffect(() => { const controller = new AbortController(); void request<DockerService>(`/services/${serviceId}`, { signal: controller.signal }).then(row => { if (row.project_id !== projectId) throw new Error('Service not found in this project.'); if (!controller.signal.aborted) setService(row) }).catch(e => { if (!controller.signal.aborted) setError(message(e)) }); return () => controller.abort() }, [projectId, serviceId])
  if (error) return <ErrorNotice error={error} />
  if (!service) return <WorkspaceSkeleton kind="service" />
  return <><ProjectBreadcrumb projectId={projectId} projectName={project?.project_name} environmentId={service.environment_id} environmentName={project?.environments.find(e => e.id === service.environment_id)?.name} serviceName={service.name} /><p className="text-sm text-muted-foreground">Access: {canManage ? 'Organization manager' : access?.permission ?? 'Checking…'}</p><ManagedService key={`${serviceId}:${access?.permission ?? canManage}`} initial={service} projectId={projectId} canManage={canManage || access?.can_develop === true} canAdmin={canManage} /><ResourceAccess resource={`/services/${serviceId}`} canManage={canManage} /></>
}
function SecretField({ label, value, secret = false }: { label: string; value: string; secret?: boolean }) {
  const [visible, setVisible] = useState(false)
  const [copied, setCopied] = useState(false)
  const [error, setError] = useState('')
  const id = `credential-${label.toLowerCase().replaceAll(' ', '-')}`
  return <Field><FieldLabel htmlFor={id}>{label}</FieldLabel><InputGroup><InputGroupInput id={id} readOnly type={secret && !visible ? 'password' : 'text'} value={value} /><InputGroupAddon align="inline-end">{secret ? <InputGroupButton aria-label={`${visible ? 'Hide' : 'Show'} ${label}`} onClick={() => setVisible(!visible)}>{visible ? <EyeOff /> : <Eye />}</InputGroupButton> : null}<InputGroupButton aria-label={`Copy ${label}`} onClick={() => { void navigator.clipboard.writeText(value).then(() => { setCopied(true); setError('') }).catch(() => setError('Could not copy. Select and copy the value manually.')) }}>{copied ? <Check /> : <Copy />}</InputGroupButton></InputGroupAddon></InputGroup><span role="status" className="text-xs empty:hidden">{error || (copied ? `${label} copied.` : '')}</span></Field>
}
function ManagedService({ initial, projectId, canManage, canAdmin }: { initial: DockerService; projectId: string; canManage: boolean; canAdmin: boolean }) {
  const [service, setService] = useState(initial)
  const [runtime, setRuntime] = useState<Runtime | null>(null)
  const [checkedAt, setCheckedAt] = useState<Date | null>(null)
  const [deployments, setDeployments] = useState<Deployment[]>([])
  const [settings, setSettings] = useState<Settings | null>(null)
  const [editing, setEditing] = useState(false)
  const [connection, setConnection] = useState(false)
  const [busy, setBusy] = useState(false)
  const [activeAction, setActiveAction] = useState<string | null>(null)
  const [queuedDeployment, setQueuedDeployment] = useState<string | null>(null)
  const actionLock = useRef(false)
  const [error, setError] = useState('')
  const [pollError, setPollError] = useState('')
  const [notice, setNotice] = useState('')
  const [rollbackTarget, setRollbackTarget] = useState<Deployment | null>(null)
  const [progress, setProgress] = useState<string | null>(null)
  const [tab, setTab] = useState('general')
  const [environment, setEnvironment] = useState<string | null>(null)
  const [environmentDirty, setEnvironmentDirty] = useState(false)
  const [confirmStop, setConfirmStop] = useState(false)
  const inFlight = useRef(false)
  const configurationButton = useRef<HTMLButtonElement>(null)
  const refresh = useCallback(async (signal?: AbortSignal) => {
    if (inFlight.current) return
    inFlight.current = true
    try { const [row, running, history] = await Promise.all([request<DockerService>(`/services/${initial.id}`, { signal }), request<Runtime>(`/services/${initial.id}/runtime`, { signal }), request<Deployment[]>(`/services/${initial.id}/deployments`, { signal })]); if (!signal?.aborted) { setService(row); setRuntime(running); setCheckedAt(new Date()); setDeployments(history); setQueuedDeployment(id => id && history.some(deployment => deployment.id === id && !['queued', 'running'].includes(deployment.status)) ? null : id); setPollError('') } }
    finally { inFlight.current = false }
  }, [initial.id])
  useEffect(() => { const controller = new AbortController(); const poll = () => { if (!document.hidden) void refresh(controller.signal).catch(e => { if (!controller.signal.aborted) setPollError(message(e)) }) }; poll(); const timer = window.setInterval(poll, 3000); document.addEventListener('visibilitychange', poll); return () => { controller.abort(); window.clearInterval(timer); document.removeEventListener('visibilitychange', poll) } }, [refresh])
  async function action(operation: () => Promise<void>, label: string | null = null) { if (actionLock.current) return; actionLock.current = true; setActiveAction(label); setBusy(true); setError(''); setNotice(''); try { await operation() } catch (e) { setError(message(e)) } finally { actionLock.current = false; setActiveAction(null); setBusy(false) } }
  function configure() { void action(async () => { setSettings(await request<Settings>(`/services/${service.id}/configuration`)); setEditing(true); setTab('configuration') }) }
  async function save(event: FormEvent<HTMLFormElement>) {
    event.preventDefault(); const element = event.currentTarget
    await action(async () => { const value = service.kind === 'application' ? { ...configurationFrom(element), ...(!canAdmin && settings ? {memory_mib:settings.memory_mib,cpus:settings.cpus,worker_mode:settings.worker_mode,command:settings.command,storage:settings.storage} : {}) } : serviceForm(element); setService(await request<DockerService>(`/services/${service.id}/configuration`, post(value))); setEditing(false); setSettings(null); setConnection(false); setEnvironment(null); setTab('general'); setNotice('Configuration saved. Deploy to apply it.') }, 'save-configuration')
  }
  function selectTab(value: string) {
    setTab(value)
    if (value === 'environment' && canManage && environment === null) void action(async () => { const result = await request<{ environment: string }>(`/services/${service.id}/environment`); setEnvironment(result.environment); setEnvironmentDirty(false) })
  }
  function control(operation: 'start' | 'stop' | 'reload') {
    void action(async () => { await request(`/services/${service.id}/control`, { ...post({ operation }), signal: AbortSignal.timeout(100_000) }); setConfirmStop(false); setNotice(operation === 'stop' ? 'Service stopped. Data is retained.' : operation === 'start' ? 'Service started. Check runtime for readiness.' : 'Service restarted using its deployed configuration.'); await refresh() }, operation)
  }
  const pending = queuedDeployment !== null || deployments.some(d => d.status === 'queued' || d.status === 'running')
  const databaseBusy = service.database_operation_pending ?? false
  const currentState = pollError ? 'unavailable' : serviceState(runtime?.state, runtime?.health)
  const deployed = !pollError && runtime && !['not_deployed', 'unavailable', 'missing'].includes(runtime.state)
  const stopped = ['exited', 'stopped', 'created'].includes(runtime?.state ?? '')
  const latest = deployments[0]
  function showProgress(id: string) { setProgress(id); setTab('deployments') }
  const Icon = types[service.kind].icon
  return <>
    <Button asChild variant="ghost" className="self-start"><Link to="/projects/$projectId" params={{ projectId }} search={{ environment: service.environment_id }}><ArrowLeft data-icon="inline-start" />Back to project</Link></Button>
    <Card><CardHeader><CardTitle><div className="flex items-center gap-3">{service.kind === 'database' ? <DatabaseLogo engine={service.database_engine ?? 'postgres'} className="size-8 shrink-0 object-contain" /> : <Icon />}<h1 className="text-2xl font-semibold tracking-tight">{service.name}</h1></div></CardTitle><CardDescription className="break-words leading-relaxed">{service.kind === 'database' ? `${databaseTypes[service.database_engine ?? 'postgres'].label} ${databaseTypes[service.database_engine ?? 'postgres'].version} · persistent storage` : service.source_type === 'github' ? `GitHub · ${service.source_repository}` : service.source_type === 'upload' ? 'Uploaded source · Dockerfile' : service.image}</CardDescription><CardAction><Badge variant="secondary">Ferrix Server</Badge></CardAction></CardHeader><CardContent><ServiceStatus state={currentState} label /><div className="mt-3 flex flex-wrap items-center gap-3"><p className="break-all text-sm text-muted-foreground">Active image: {service.active_image ?? 'No active deployment'}</p>{service.kind === 'application' && service.traffic_enabled !== false && service.automatic_url && runtime?.state === 'running' ? <Button asChild variant="outline" size="sm"><a href={service.automatic_url} target="_blank" rel="noopener noreferrer">Open website<ArrowUpRight data-icon="inline-end" /></a></Button> : null}</div><p className="mt-2 text-xs text-muted-foreground">{pollError ? 'Runtime status unavailable. Retrying automatically…' : checkedAt ? `Status checked ${checkedAt.toLocaleTimeString()} · refreshes every 3 seconds` : 'Checking runtime status…'}</p></CardContent></Card>
    <Dialog open={rollbackTarget !== null} onOpenChange={open => { if (!open && !busy) setRollbackTarget(null) }}><DialogContent><DialogHeader><DialogTitle>Rollback {service.name}?</DialogTitle><DialogDescription>This creates a new deployment with the selected version’s exact image, port and environment variables. Command, runtime mode and storage settings also come from that deployment. Volume contents are not rolled back. Saved configuration stays available for your next regular deploy.</DialogDescription></DialogHeader>{rollbackTarget ? <div className="flex flex-col gap-2 text-sm"><p className="break-all">{rollbackTarget.image}</p><p>{new Date(rollbackTarget.created_at).toLocaleString()}</p><p className="break-all text-muted-foreground">{rollbackTarget.image_id}</p><p>The original image must still exist on this Docker server. External data and database changes are not restored.</p></div> : null}<DialogFooter><Button variant="outline" disabled={busy} onClick={() => setRollbackTarget(null)}>Cancel</Button><Button disabled={busy || pending} onClick={() => void action(async () => { if (!rollbackTarget) return; const queued = await request<{ id: string }>(`/services/${service.id}/rollback`, post({ deployment_id: rollbackTarget.id })); setRollbackTarget(null); setProgress(queued.id); setTab('deployments'); setNotice('Rollback queued.'); await refresh() })}>{busy ? 'Queuing…' : 'Confirm rollback'}</Button></DialogFooter>{error ? <p role="alert" className="text-sm text-destructive">{error}</p> : null}</DialogContent></Dialog>
    <ActionNotice message={notice} onDismiss={() => setNotice('')} /><ErrorNotice error={error || pollError} />
    {databaseBusy ? <Alert><CircleAlert /><AlertTitle>Backup or restore in progress</AlertTitle><AlertDescription><p>Follow backup and restore status before changing this service.</p><Button variant="outline" size="sm" onClick={() => setTab('backups')}>View backups</Button></AlertDescription></Alert> : null}
    {service.cutover_pending?<Alert><AlertTitle>Worker waiting for project cutover</AlertTitle><AlertDescription>Validate the restored project, then activate it from Project backups. This worker remains stopped until cutover.</AlertDescription></Alert>:service.traffic_enabled===false?<Alert><AlertTitle>Isolated restore · public traffic disabled</AlertTitle><AlertDescription>Start the web service to validate it through its server-only loopback port, then use Project backups to cut over.</AlertDescription></Alert>:null}
    {service.restore_blocked ? <Alert variant="destructive"><AlertTitle>Restore not complete</AlertTitle><AlertDescription>This target stays stopped. Follow Backups for progress. If restore failed, delete this target and restore into a new service.</AlertDescription></Alert> : null}
    {service.pending_changes ? <Alert><CircleAlert /><AlertTitle>Changes pending deployment</AlertTitle><AlertDescription>Saved configuration differs from the active deployment. Deploy to apply it.</AlertDescription></Alert> : null}
    {latest && (pending || latest.status === 'failed') ? <Alert><CircleAlert /><AlertTitle>{pending ? 'Deployment in progress' : 'Latest deployment failed'}</AlertTitle><AlertDescription><p>{pending ? latest.status === 'queued' ? 'Waiting for the deployment worker…' : latest.logs.trim().split('\n').at(-1) || 'Starting deployment…' : service.active_deployment_id && runtime?.state === 'running' ? 'The previous deployment is still running.' : 'Review deployment output to resolve the failure.'}</p><Button variant="outline" size="sm" onClick={() => showProgress(latest.id)}>{pending ? 'View progress' : 'View deployment logs'}</Button></AlertDescription></Alert> : null}
    <Tabs value={tab} onValueChange={selectTab} className="gap-4"><div className="max-w-full overflow-x-auto pb-1"><TabsList aria-label="Service settings"><TabsTrigger value="general">Overview</TabsTrigger>{service.kind === 'application' ? <TabsTrigger value="source">Source</TabsTrigger> : null}<TabsTrigger value="configuration">Configuration</TabsTrigger><TabsTrigger value="environment">Environment</TabsTrigger>{(service.database_engine === 'mariadb' && canAdmin || service.kind === 'application' && canManage) && <TabsTrigger value="database">Database</TabsTrigger>}{service.kind === 'application' && !service.worker_mode ? <TabsTrigger value="domains">Domains</TabsTrigger> : null}{(service.kind === 'application' || (service.kind === 'database' && ['postgres', 'mariadb'].includes(service.database_engine ?? 'postgres'))) ? <TabsTrigger value="backups">Backups</TabsTrigger> : null}<TabsTrigger value="deployments">Deployments</TabsTrigger><TabsTrigger value="logs">Logs</TabsTrigger><TabsTrigger value="monitoring">Monitoring</TabsTrigger><TabsTrigger value="notes">Notes</TabsTrigger></TabsList></div>
      <TabsContent value="general" className="flex flex-col gap-4">
        <Card><CardHeader><CardTitle><h2>Deploy Settings</h2></CardTitle><CardDescription>Deploy applies saved changes. Reload restarts the existing containers without pulling a new image.</CardDescription></CardHeader><CardContent className="flex flex-col gap-4"><div className="flex flex-wrap gap-3">
          {canManage ? <><LoadingButton loading={activeAction === 'deploy' || pending} loadingText="Deploying…" disabled={busy || databaseBusy || (service.restore_blocked || service.cutover_pending) || editing || environmentDirty || pending || confirmStop} onClick={() => void action(async () => { const queued = await request<{ id: string }>(`/services/${service.id}/deploy`, post({})); setQueuedDeployment(queued.id); setProgress(queued.id); setSettings(null); setConnection(false); setNotice('Deployment queued.'); setTab('deployments'); await refresh() }, 'deploy')}><Rocket data-icon="inline-start" />{pending ? 'Deploying…' : 'Deploy'}</LoadingButton><LoadingButton loading={activeAction === 'reload'} loadingText="Restarting…" variant="secondary" disabled={busy || databaseBusy || (service.restore_blocked || service.cutover_pending) || pending || !deployed || stopped || confirmStop} onClick={() => control('reload')}><RefreshCw data-icon="inline-start" />Reload</LoadingButton>{stopped ? <Button variant="outline" disabled={busy || databaseBusy || (service.restore_blocked || service.cutover_pending) || pending || !deployed} onClick={() => control('start')}><Play data-icon="inline-start" />Start</Button> : <Button variant="destructive" disabled={busy || databaseBusy || pending || !deployed} onClick={() => setConfirmStop(true)}><Square data-icon="inline-start" />Stop</Button>}<ServiceTerminal serviceId={service.id} name={service.name} disabled={busy || pending || runtime?.state !== 'running'} /><Button ref={configurationButton} variant="outline" onClick={configure} disabled={busy || editing || environmentDirty}>Configuration</Button></> : null}
          {canAdmin ? <ServiceTransfer serviceId={service.id} projectId={projectId} environmentId={service.environment_id} name={service.name} disabled={busy || pending || databaseBusy || editing || environmentDirty || confirmStop || !!service.restore_blocked || !!service.cutover_pending} /> : null}
          <Button variant="outline" disabled={busy || !deployed} onClick={() => { setTab('logs') }}>Container logs</Button>
        </div>{confirmStop ? <Alert><CircleAlert /><AlertTitle>Stop {service.name}?</AlertTitle><AlertDescription><p>The service will be unavailable until you start it again. Stored data is retained.</p><div className="mt-3 flex flex-wrap gap-2"><Button variant="outline" disabled={busy} onClick={() => setConfirmStop(false)}>Cancel</Button><Button variant="destructive" disabled={busy} onClick={() => control('stop')}>{busy ? 'Stopping…' : 'Confirm stop'}</Button></div></AlertDescription></Alert> : null}</CardContent></Card>
        {service.kind === 'application' && service.automatic_url ? <Card><CardHeader><CardTitle><h2>Automatic domain</h2></CardTitle><CardDescription>A stable address routed by Caddy to the active deployment.</CardDescription></CardHeader><CardContent className="flex flex-col gap-3"><p className="break-all text-sm">{service.automatic_url}</p>{runtime?.state === 'running' ? <Button asChild variant="outline" className="self-start"><a href={service.automatic_url} target="_blank" rel="noopener noreferrer">Open website<ArrowUpRight data-icon="inline-end" /></a></Button> : <p className="text-sm text-muted-foreground">Deploy or start this application to open its website.</p>}<p className="text-xs text-muted-foreground">Applications deployed before automatic domains were enabled need a redeploy to join the ingress network.</p></CardContent></Card> : null}
        <Card><CardHeader><CardTitle><h2>Runtime</h2></CardTitle><CardDescription className="break-words leading-relaxed">{service.kind === 'database' ? 'Private database on the environment network. No host port is published.' : 'The address belongs to the Docker host.'}</CardDescription><CardAction><Badge variant="outline">{currentState.replaceAll('_', ' ')}</Badge></CardAction></CardHeader><CardContent className="flex flex-col gap-3">
          {service.kind === 'application' ? runtime?.state === 'running' && runtime.host_port ? <Button asChild variant="outline" className="self-start"><a href={`http://127.0.0.1:${runtime.host_port}`} target="_blank" rel="noopener noreferrer">Open http://127.0.0.1:{runtime.host_port}<ArrowUpRight data-icon="inline-end" /></a></Button> : <p className="text-sm text-muted-foreground">Start or deploy the application to open its address.</p> : null}
          {service.kind === 'database' ? <><p className="break-all text-sm">Internal host: {runtime?.internal_host ?? `ferrix-${service.id.replaceAll('-', '')}` }:{databaseTypes[service.database_engine ?? 'postgres'].port}</p>{canManage ? <Button variant="outline" className="self-start" disabled={busy} onClick={() => void action(async () => { setSettings(await request<Settings>(`/services/${service.id}/configuration`)); setConnection(true) })}>Connection details</Button> : null}</> : null}
        </CardContent></Card>
        {connection && settings ? <Card><CardHeader><CardTitle><h2>Internal Credentials</h2></CardTitle><CardDescription>Connect from an application in the same environment. Redeploy older applications to join the environment network.</CardDescription></CardHeader><CardContent><FieldGroup className="grid gap-4 sm:grid-cols-2">{service.database_engine !== 'redis' ? <><SecretField label="User" value={settings.username ?? ''} /><SecretField label="Database Name" value={settings.database_name ?? ''} /></> : null}<SecretField label="Password" value={settings.password ?? ''} secret />{settings.root_password ? <SecretField label="Root Password" value={settings.root_password} secret /> : null}<SecretField label="Connection URL" value={settings.connection_uri ?? ''} secret /></FieldGroup></CardContent></Card> : null}
      </TabsContent>
      {service.kind === 'application' ? <TabsContent value="source"><ApplicationSource serviceId={service.id} canManage={canManage} onSaved={refresh} /></TabsContent> : null}
      <TabsContent value="configuration" forceMount hidden={tab !== 'configuration'} className="flex flex-col gap-4 data-[state=inactive]:hidden">
        {!editing && <Card><CardHeader><CardTitle><h2>Configuration</h2></CardTitle><CardDescription>Saved settings apply on the next deployment.</CardDescription></CardHeader><CardContent>{canManage ? <Button disabled={busy || environmentDirty} onClick={configure}>Load configuration</Button> : <p>A developer assignment is required to edit application configuration.</p>}</CardContent></Card>}
        {editing && settings ? <Card><CardHeader><CardTitle><h2>Service configuration</h2></CardTitle><CardDescription className="break-words leading-relaxed">{service.kind === 'database' ? 'Credentials are locked after the first deployment. Data survives redeploys.' : 'Save changes, then deploy to apply them.'}</CardDescription></CardHeader><CardContent><form onSubmit={save} className="flex flex-col gap-4">{service.kind === 'application' ? <ConfigurationFields runtimeLocked={!canAdmin} source={service.source_type} initial={{ memory_mib: settings.memory_mib, cpus: settings.cpus, storage_volume: settings.storage_volume, worker_mode: settings.worker_mode, command: settings.command, storage: settings.storage, name: settings.name, image: settings.image ?? service.image, container_port: settings.container_port ?? 80, environment: settings.environment ?? '' }} disabled={busy} /> : <ServiceFields settings={settings} busy={busy} />}<div className="flex justify-end gap-2"><Button type="button" variant="outline" disabled={busy} onClick={() => { setEditing(false); setSettings(null); setConnection(false); setTab('general') }}>Cancel</Button><LoadingButton loading={activeAction === 'save-configuration'} loadingText="Saving…" disabled={busy}>{busy ? 'Saving…' : 'Save configuration'}</LoadingButton></div></form></CardContent></Card> : null}
      </TabsContent>
      <TabsContent value="deployments" className="flex flex-col gap-4">
        <Card><CardHeader><CardTitle><h2>Deployments</h2></CardTitle><CardDescription>Latest 30 deployments. Progress refreshes automatically.</CardDescription></CardHeader><CardContent className="flex flex-col gap-4">{deployments.length ? <DataTable label="Deployment history" data={deployments} initialSorting={[{ id: 'created', desc: true }]} columns={[
          { id: 'status', label: 'Status', value: deployment => deployment.status, cell: deployment => <Badge variant={deployment.status === 'failed' ? 'destructive' : deployment.status === 'succeeded' ? 'secondary' : 'outline'}>{deployment.status}{deployment.rollback_of ? ' · Rollback' : ''}{deployment.id === service.active_deployment_id ? ' · Active' : ''}</Badge> },
          { id: 'image', label: 'Image', value: deployment => `${deployment.source_repository ?? deployment.image} ${deployment.source_branch ?? ''} ${deployment.git_commit ?? ''}`, cell: deployment => deployment.source_archive_sha256 ? <div className="flex min-w-40 flex-col gap-1"><span>Uploaded source</span><span className="text-xs text-muted-foreground" title={deployment.source_archive_sha256}>SHA-256 · {deployment.source_archive_sha256.slice(0,12)}</span></div> : deployment.source_repository ? <div className="flex min-w-40 flex-col gap-1"><span className="break-all">{deployment.source_repository}</span><span className="text-xs text-muted-foreground">{deployment.source_branch} · {deployment.git_commit?.slice(0,12) ?? 'Resolving commit…'}</span></div> : <span className="break-all" title={deployment.image_id ?? undefined}>{deployment.image}</span> },
          { id: 'initiator', label: 'Started by', value: deployment => `${deployment.requested_by_name ?? 'Unknown user'} ${deployment.requested_by_key_name ?? ''} ${deployment.trigger_kind === 'webhook' ? `GitHub push @${deployment.trigger_sender ?? ''}` : ''}`, cell: deployment => <div className="min-w-32"><p>{deployment.requested_by_name ?? 'Unknown user'}</p>{deployment.trigger_kind === 'webhook' ? <p className="text-xs text-muted-foreground">GitHub push{deployment.trigger_sender ? ` · @${deployment.trigger_sender}` : ''}</p> : null}{deployment.requested_by_key_name ? <p className="text-xs text-muted-foreground">API key · {deployment.requested_by_key_name}</p> : null}</div> },
          { id: 'created', label: 'Created', value: deployment => deployment.created_at, cell: deployment => <time dateTime={deployment.created_at}>{new Date(deployment.created_at).toLocaleString()}</time> },
          ...(service.kind === 'application' && canAdmin ? [{ id: 'rollback', label: 'Rollback', cell: (deployment: Deployment) => deployment.status === 'succeeded' && deployment.image_id ? <Button variant="outline" size="sm" disabled={busy || pending || editing || environmentDirty || confirmStop || deployment.id === service.active_deployment_id} onClick={() => setRollbackTarget(deployment)}>Rollback</Button> : null }] : []),
          { id: 'progress', label: 'Progress', cell: deployment => <Button variant="ghost" aria-expanded={progress === deployment.id} onClick={() => setProgress(current => current === deployment.id ? null : deployment.id)}>Progress</Button> },
        ]} /> : <Empty><EmptyHeader><EmptyMedia variant="icon"><Rocket /></EmptyMedia><EmptyTitle>Ready to deploy</EmptyTitle><EmptyDescription>Save your configuration, then deploy this service.</EmptyDescription></EmptyHeader></Empty>}</CardContent></Card>
        {progress ? <Card><CardHeader><CardTitle><h2>Deployment progress</h2></CardTitle><CardDescription>{progress}</CardDescription><CardAction><Button variant="ghost" onClick={() => setProgress(null)}>Close progress</Button></CardAction></CardHeader><CardContent><pre className="max-h-64 overflow-auto whitespace-pre-wrap break-all text-xs">{deployments.find(deployment => deployment.id === progress)?.logs || 'Waiting for worker…'}</pre></CardContent></Card> : null}
      </TabsContent>
      <TabsContent value="environment" forceMount hidden={tab !== 'environment'} className="data-[state=inactive]:hidden"><Card><CardHeader><CardTitle><h2>Environment variables</h2></CardTitle><CardDescription>Values are encrypted at rest and applied on the next deploy. Reload uses the existing configuration.</CardDescription></CardHeader><CardContent>{canManage ? environment === null ? <Button variant="outline" disabled={busy || editing} onClick={() => void action(async () => { const result = await request<{ environment: string }>(`/services/${service.id}/environment`); setEnvironment(result.environment) })}>Load environment</Button> : <form className="flex flex-col gap-4" onSubmit={event => { event.preventDefault(); void action(async () => { await request(`/services/${service.id}/environment`, post({ environment })); setEnvironmentDirty(false); setNotice('Environment saved. Deploy to apply it.') }, 'save-environment') }}><FieldGroup><Field><FieldLabel htmlFor="service-environment">Environment variables</FieldLabel><Textarea id="service-environment" value={environment} onChange={event => { setEnvironment(event.target.value); setEnvironmentDirty(true) }} disabled={busy || editing} rows={8} maxLength={16384} spellCheck={false} /><FieldDescription>{service.kind === 'database' ? 'Database credential and initialization variables are managed by Ferrix. Other variables, such as TZ, can be set here.' : 'One KEY=value per line. Values are passed literally without shell expansion.'}</FieldDescription></Field></FieldGroup><LoadingButton loading={activeAction === 'save-environment'} loadingText="Saving…" className="self-end" disabled={busy || editing || !environmentDirty}>Save environment</LoadingButton></form> : <p className="text-sm text-muted-foreground">A developer assignment is required to view or edit application environment values.</p>}</CardContent></Card></TabsContent>
      {(service.kind === 'application' || (service.kind === 'database' && ['postgres', 'mariadb'].includes(service.database_engine ?? 'postgres'))) ? <TabsContent value="backups">{canAdmin ? <ServiceBackups serviceId={service.id} serviceName={service.name} canManage={canAdmin} application={service.kind === 'application'} /> : <p>Backups and restores are managed by your organization managers.</p>}</TabsContent> : null}
      {service.kind === 'application' && !service.worker_mode ? <TabsContent value="domains">{canAdmin ? <ServiceDomains serviceId={service.id} canManage={canAdmin} /> : <p>Domains are managed by your organization managers.</p>}</TabsContent> : null}
      <TabsContent value="database">{service.kind === 'application' && <PhpTemplateStatus serviceId={service.id} canManage={canAdmin} />}{service.database_engine === 'mariadb' && canAdmin ? <ManagedDatabases serviceId={service.id} environmentId={service.environment_id} /> : service.kind === 'application' && canManage ? <ApplicationDatabase serviceId={service.id} /> : null}</TabsContent>
      <TabsContent value="logs"><ServiceLogs serviceId={service.id} onClose={() => setTab('general')} /></TabsContent>
      <TabsContent value="monitoring"><ServiceMetrics serviceId={service.id} /></TabsContent>
      <TabsContent value="notes" forceMount hidden={tab !== 'notes'} className="data-[state=inactive]:hidden"><ServiceNotes serviceId={service.id} canManage={canManage} /></TabsContent>
    </Tabs>
  </>
}
