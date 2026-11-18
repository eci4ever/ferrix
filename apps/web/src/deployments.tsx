import { useCallback, useEffect, useRef, useState } from 'react'
import type { FormEvent } from 'react'
import { Link } from '@tanstack/react-router'
import { ArrowLeft, ArrowUpRight, RefreshCw, Rocket, CircleAlert } from 'lucide-react'
import { request } from '@/lib/api'
import { Alert, AlertDescription, AlertTitle } from '@/components/ui/alert'
import { Badge } from '@/components/ui/badge'
import { Button } from '@/components/ui/button'
import { Card, CardHeader, CardTitle, CardDescription, CardContent, CardAction } from '@/components/ui/card'
import { Field, FieldGroup, FieldLabel, FieldDescription } from '@/components/ui/field'
import { Input } from '@/components/ui/input'
import { Textarea } from '@/components/ui/textarea'
import { Skeleton } from '@/components/ui/skeleton'
import { Empty, EmptyHeader, EmptyMedia, EmptyTitle, EmptyDescription } from '@/components/ui/empty'

type Application = { id: string; environment_id: string; project_id: string; name: string; image: string; container_port: number; active_container_id: string | null }
type Configuration = { name: string; image: string; container_port: number; environment: string }
type Deployment = { id: string; status: 'queued' | 'running' | 'succeeded' | 'failed'; image: string; logs: string; created_at: string; image_id: string | null }
type Runtime = { state: string; host_port: number | null }
const json = (body: unknown) => ({ method: 'POST', headers: { 'Content-Type': 'application/json' }, body: JSON.stringify(body) })
const message = (error: unknown) => error instanceof Error ? error.message : 'The request could not be completed.'
function ErrorNotice({ error }: { error: string }) { return error ? <Alert variant="destructive"><CircleAlert /><AlertTitle>Something needs attention</AlertTitle><AlertDescription>{error}</AlertDescription></Alert> : null }

export function ConfigurationFields({ initial, disabled }: { initial?: Configuration; disabled: boolean }) {
  return <FieldGroup>
    <Field><FieldLabel htmlFor="application-name">Application name</FieldLabel><Input id="application-name" name="name" required maxLength={80} autoFocus defaultValue={initial?.name ?? ''} placeholder="e.g. nginx" disabled={disabled} /></Field>
    <Field><FieldLabel htmlFor="application-image">Docker image</FieldLabel><Input id="application-image" name="image" required maxLength={255} defaultValue={initial?.image ?? 'nginx:alpine'} spellCheck={false} disabled={disabled} /><FieldDescription>Use a public registry image, for example nginx:alpine. Registry credentials and Git builds are not available yet.</FieldDescription></Field>
    <Field><FieldLabel htmlFor="application-port">Container port</FieldLabel><Input id="application-port" name="container_port" type="number" min={1} max={65535} required defaultValue={initial?.container_port ?? 80} disabled={disabled} /><FieldDescription>Ferrix allocates a free port on the server's loopback address.</FieldDescription></Field>
    <Field><FieldLabel htmlFor="application-environment">Environment variables</FieldLabel><Textarea id="application-environment" name="environment" rows={5} maxLength={16384} defaultValue={initial?.environment ?? ''} placeholder={'KEY=value\nANOTHER_KEY=value'} spellCheck={false} disabled={disabled} /><FieldDescription>One KEY=value per line. Values are stored encrypted and passed literally; shell expressions are not expanded.</FieldDescription></Field>
  </FieldGroup>
}
export function configurationFrom(form: HTMLFormElement): Configuration {
  const data = new FormData(form)
  return { name: String(data.get('name') ?? ''), image: String(data.get('image') ?? ''), container_port: Number(data.get('container_port')), environment: String(data.get('environment') ?? '') }
}

export function ApplicationWorkspace({ projectId, applicationId, canManage }: { projectId: string; applicationId: string; canManage: boolean }) {
  const [app, setApp] = useState<Application | null>(null)
  const [deployments, setDeployments] = useState<Deployment[]>([])
  const [runtime, setRuntime] = useState<Runtime | null>(null)
  const [config, setConfig] = useState<Configuration | null>(null)
  const [error, setError] = useState('')
  const [notice, setNotice] = useState('')
  const [pollError, setPollError] = useState('')
  const [busy, setBusy] = useState(false)
  const [editing, setEditing] = useState(false)
  const [log, setLog] = useState<string | null>(null)
  const [expanded, setExpanded] = useState<string | null>(null)
  const busyRef = useRef(false)
  const editButton = useRef<HTMLButtonElement>(null)
  const refresh = useCallback(async (signal?: AbortSignal) => {
    if (busyRef.current) return
    busyRef.current = true
    try {
      const [next, history, running] = await Promise.all([request<Application>(`/applications/${applicationId}`, { signal }), request<Deployment[]>(`/applications/${applicationId}/deployments`, { signal }), request<Runtime>(`/applications/${applicationId}/runtime`, { signal })])
      if (next.project_id !== projectId) throw new Error('Application not found in this project.')
      if (!signal?.aborted) { setApp(next); setDeployments(history); setRuntime(running) }
    } finally { busyRef.current = false }
  }, [applicationId, projectId])
  useEffect(() => {
    const controller = new AbortController()
    async function poll() { if (document.hidden) return; try { await refresh(controller.signal); if (!controller.signal.aborted) setPollError('') } catch (e) { if (!controller.signal.aborted) setPollError(message(e)) } }
    void poll()
    const interval = window.setInterval(() => { void poll() }, 3000)
    document.addEventListener('visibilitychange', poll)
    return () => { controller.abort(); window.clearInterval(interval); document.removeEventListener('visibilitychange', poll) }
  }, [refresh])
  const pending = deployments.some(d => d.status === 'queued' || d.status === 'running')
  async function action(operation: () => Promise<void>) { setBusy(true); setError(''); setNotice(''); try { await operation() } catch (e) { setError(message(e)) } finally { setBusy(false) } }
  function edit() { void action(async () => { setConfig(await request<Configuration>(`/applications/${applicationId}/configuration`)); setEditing(true) }) }
  async function save(event: FormEvent<HTMLFormElement>) {
    event.preventDefault(); const configuration = configurationFrom(event.currentTarget)
    await action(async () => { setApp(await request<Application>(`/applications/${applicationId}/configuration`, json(configuration))); setEditing(false); setConfig(null); setNotice('Configuration saved. Deploy to apply these settings.'); editButton.current?.focus() })
  }
  function deploy() { void action(async () => { await request(`/applications/${applicationId}/deploy`, json({})); setNotice('Deployment queued. Progress appears below.'); await refresh() }) }
  function containerLogs() { void action(async () => { const result = await request<{ logs: string }>(`/applications/${applicationId}/logs`); setLog(result.logs || 'No output yet.') }) }
  return <>
    <Button asChild variant="ghost" className="self-start"><Link to="/projects/$projectId" params={{ projectId }}><ArrowLeft data-icon="inline-start" />Back to project</Link></Button>
    <div className="flex flex-wrap items-center justify-between gap-4"><div><h1 className="text-2xl font-semibold tracking-tight">{app?.name ?? 'Application'}</h1><p className="text-sm text-muted-foreground">{app?.image ?? 'Loading configuration…'}</p></div><div className="flex flex-wrap gap-2"><Button variant="outline" onClick={containerLogs} disabled={busy || !app?.active_container_id}>Container logs</Button>{canManage ? <><Button ref={editButton} variant="outline" onClick={edit} disabled={busy || !app}>Configuration</Button><Button onClick={deploy} disabled={busy || pending || !app || editing}><Rocket data-icon="inline-start" />{pending ? 'Deploying…' : app?.active_container_id ? 'Redeploy' : 'Deploy'}</Button></> : null}</div></div>
    <p role="status" className="text-sm empty:hidden">{notice}</p><ErrorNotice error={error || pollError} />
    {app ? <Card><CardHeader><CardTitle><h2>Runtime</h2></CardTitle><CardDescription>The current container and its access address on the Docker server.</CardDescription><CardAction><Badge variant={runtime?.state === 'running' ? 'secondary' : 'outline'}>{runtime?.state ?? 'Checking…'}</Badge></CardAction></CardHeader><CardContent className="flex flex-col gap-2"><p>Configured container port: {app.container_port}</p>{runtime?.host_port ? <><Button asChild variant="outline" className="self-start"><a href={`http://127.0.0.1:${runtime.host_port}`} target="_blank" rel="noopener noreferrer">Open http://127.0.0.1:{runtime.host_port}<ArrowUpRight data-icon="inline-end" /></a></Button><p className="text-sm text-muted-foreground">This loopback address belongs to the Docker server. For a remote VPS, use an SSH tunnel. Domains and public HTTPS routing come later.</p></> : <p className="text-sm text-muted-foreground">Deploy this application to allocate a server port.</p>}</CardContent></Card> : !error ? <Skeleton className="h-40 w-full" /> : null}
    {editing && config ? <Card><CardHeader><CardTitle><h2>Application configuration</h2></CardTitle><CardDescription>Saved settings are applied by the next deployment. An empty variables field clears existing variables.</CardDescription></CardHeader><CardContent><form onSubmit={save} className="flex flex-col gap-4"><ConfigurationFields initial={config} disabled={busy} /><div className="flex justify-end gap-2"><Button type="button" variant="outline" disabled={busy} onClick={() => { setEditing(false); setConfig(null); editButton.current?.focus() }}>Cancel</Button><Button disabled={busy}>{busy ? 'Saving…' : 'Save configuration'}</Button></div></form></CardContent></Card> : null}
    {log !== null ? <Card><CardHeader><CardTitle><h2>Container logs</h2></CardTitle><CardDescription>Last 100 lines, up to 64 KiB.</CardDescription><CardAction><Button variant="ghost" onClick={() => setLog(null)}>Close</Button></CardAction></CardHeader><CardContent><pre className="max-h-80 overflow-auto whitespace-pre-wrap break-all text-xs">{log}</pre></CardContent></Card> : null}
    <Card><CardHeader><CardTitle><h2>Deployments</h2></CardTitle><CardDescription>Latest 30 deployments. Progress refreshes automatically.</CardDescription><CardAction><RefreshCw aria-hidden="true" /></CardAction></CardHeader><CardContent className="flex flex-col gap-4">{deployments.length ? deployments.map(deployment => <div key={deployment.id} className="flex flex-col gap-2"><div className="flex flex-wrap items-center gap-3"><Badge variant={deployment.status === 'failed' ? 'destructive' : deployment.status === 'succeeded' ? 'secondary' : 'outline'}>{deployment.status}</Badge><span className="min-w-0 break-all text-sm">{deployment.image}</span><time className="text-xs text-muted-foreground" dateTime={deployment.created_at}>{new Date(deployment.created_at).toLocaleString()}</time><Button variant="ghost" onClick={() => setExpanded(expanded === deployment.id ? null : deployment.id)} aria-expanded={expanded === deployment.id}>Progress</Button></div>{expanded === deployment.id || deployment.status === 'running' || deployment.status === 'failed' ? <pre className="max-h-64 overflow-auto whitespace-pre-wrap break-all text-xs">{deployment.logs || 'Waiting for worker…'}</pre> : null}{expanded === deployment.id && deployment.image_id ? <p className="break-all text-xs text-muted-foreground">Image: {deployment.image_id}</p> : null}</div>) : <Empty><EmptyHeader><EmptyMedia variant="icon"><Rocket /></EmptyMedia><EmptyTitle>Ready for your first deployment</EmptyTitle><EmptyDescription>Deploy pulls the image, starts a container and records the result here.</EmptyDescription></EmptyHeader></Empty>}</CardContent></Card>
  </>
}
