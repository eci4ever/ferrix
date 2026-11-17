import { StrictMode, useRef, useState } from 'react'
import type { FormEvent } from 'react'
import { createRoot } from 'react-dom/client'
import { createRootRoute, createRoute, createRouter, RouterProvider, Outlet, Link, useRouter } from '@tanstack/react-router'
import { Box, Layers3, Server, RefreshCw, Plus, Folder, CheckCircle2, CircleAlert } from 'lucide-react'
import { Button } from '@/components/ui/button'
import { Badge } from '@/components/ui/badge'
import { Input } from '@/components/ui/input'
import { Field, FieldGroup, FieldLabel, FieldDescription } from '@/components/ui/field'
import { Table, TableHeader, TableHead, TableBody, TableRow, TableCell, TableCaption } from '@/components/ui/table'
import { Alert, AlertTitle, AlertDescription } from '@/components/ui/alert'
import { Empty, EmptyHeader, EmptyMedia, EmptyTitle, EmptyDescription } from '@/components/ui/empty'
import { Skeleton } from '@/components/ui/skeleton'
import { load, request } from '@/lib/api'
import type { Project, SystemStatus, Container } from '@/lib/api'
import './index.css'

function ErrorNotice({ message }: { message: string }) {
  return <Alert variant="destructive"><CircleAlert /><AlertTitle>Something needs attention</AlertTitle><AlertDescription>{message}</AlertDescription></Alert>
}

function Shell() {
  const { status } = rootRoute.useLoaderData()
  const router = useRouter()
  const [refreshing, setRefreshing] = useState(false)
  const [refreshError, setRefreshError] = useState('')
  async function refresh() {
    setRefreshing(true)
    setRefreshError('')
    try { await router.invalidate() }
    catch { setRefreshError('Refresh failed. Please try again.') }
    finally { setRefreshing(false) }
  }
  return <div className="min-h-svh bg-background text-foreground md:grid md:grid-cols-[240px_minmax(0,1fr)]">
    <a href="#main" className="sr-only focus:not-sr-only focus:absolute focus:bg-background focus:p-4">Skip to content</a>
    <aside className="flex flex-col gap-6 border-b bg-sidebar p-4 text-sidebar-foreground md:border-r md:border-b-0">
      <Link to="/" className="flex items-center gap-2 px-2 text-lg font-semibold" aria-label="Ferrix home"><span className="flex size-8 items-center justify-center rounded-lg bg-primary text-primary-foreground [&_svg]:size-5"><Layers3 aria-hidden="true" /></span>ferrix</Link>
      <p className="px-2 text-xs font-medium text-muted-foreground">Workspace</p>
      <nav aria-label="Main navigation" className="flex gap-2 md:flex-col">
        <Button variant="ghost" asChild className="justify-start"><Link to="/" activeOptions={{ exact: true }} activeProps={{ className: "bg-sidebar-accent text-sidebar-accent-foreground" }}><Folder data-icon="inline-start" />Projects</Link></Button>
        <Button variant="ghost" asChild className="justify-start"><Link to="/containers" activeProps={{ className: "bg-sidebar-accent text-sidebar-accent-foreground" }}><Box data-icon="inline-start" />Containers</Link></Button>
      </nav>
      <div className="mt-auto hidden items-center gap-2 px-2 py-4 text-sm text-muted-foreground md:flex [&_svg]:size-4 [&_strong]:block [&_span]:block [&_span]:text-xs"><Server aria-hidden="true" /><div><strong>Local server</strong><span>Development workspace</span></div></div>
    </aside>
    <div className="flex min-w-0 flex-col">
      <header className="flex h-16 items-center justify-between gap-4 border-b px-4 text-sm md:px-6"><span>Local workspace <span className="text-muted-foreground">/ Ferrix</span></span><Button variant="outline" onClick={refresh} disabled={refreshing}><RefreshCw data-icon="inline-start" />{refreshing ? 'Refreshing…' : 'Refresh'}</Button></header>
      <main id="main" className="mx-auto flex w-full max-w-7xl flex-col gap-6 p-4 md:p-6" tabIndex={-1}>
        {status.data === null ? <ErrorNotice message={status.error} /> : <div className="flex flex-wrap items-center gap-3 text-sm [&>span:first-child]:flex [&>span:first-child]:items-center [&>span:first-child]:gap-2 [&>span:first-child]:text-muted-foreground [&_svg]:size-4" aria-label="Service connections">
          <span><Server aria-hidden="true" />Connected services</span>
          <Connection name="PostgreSQL" connected={status.data.database} />
          <Connection name="Docker" connected={status.data.docker} />
          <span className="ml-auto text-xs text-muted-foreground">v{status.data.version}</span>
        </div>}
        {refreshError ? <ErrorNotice message={refreshError} /> : null}
        <Outlet />
      </main>
      <footer className="mt-auto flex flex-wrap justify-between gap-3 p-6 text-xs text-muted-foreground">Ferrix <span>Self-hosted deployment workspace</span></footer>
    </div>
  </div>
}

function Connection({ name, connected }: { name: string; connected: boolean }) {
  return <Badge variant={connected ? 'secondary' : 'outline'}>{connected ? <CheckCircle2 data-icon="inline-start" /> : <CircleAlert data-icon="inline-start" />}{name}: {connected ? 'Connected' : 'Unavailable'}</Badge>
}

function ProjectsPage() {
  const { projects } = projectsRoute.useLoaderData()
  const router = useRouter()
  const [creating, setCreating] = useState(false)
  const [saving, setSaving] = useState(false)
  const [error, setError] = useState('')
  const [notice, setNotice] = useState('')
  const createButton = useRef<HTMLButtonElement>(null)

  function closeForm() { setCreating(false); setError(''); createButton.current?.focus() }
  async function createProject(event: FormEvent<HTMLFormElement>) {
    event.preventDefault()
    const form = new FormData(event.currentTarget)
    const name = String(form.get('name') ?? '').trim()
    if (!name) { setError('Enter a project name.'); return }
    setSaving(true); setError(''); setNotice('')
    try {
      await request<Project>('/projects', { method: 'POST', headers: { 'Content-Type': 'application/json' }, body: JSON.stringify({ name, description: form.get('description') }) })
      closeForm()
      setNotice(`Project “${name}” created.`)
      await router.invalidate()
    } catch (error) { setError(error instanceof Error ? error.message : 'Could not create project.') }
    finally { setSaving(false) }
  }
  return <>
    <div className="flex flex-wrap items-center justify-between gap-4 [&_h1]:text-2xl [&_h1]:font-semibold [&_h1]:tracking-tight [&_p]:mt-1 [&_p]:text-sm [&_p]:text-muted-foreground"><div><h1>Projects</h1><p>A home for the applications you deploy.</p></div><Button ref={createButton} size="lg" onClick={() => { setCreating(true); setNotice('') }} aria-expanded={creating}><Plus data-icon="inline-start" />New project</Button></div>
    <p role="status" className="text-sm text-foreground empty:hidden">{notice}</p>
    {creating ? <section className="grid gap-6 rounded-xl border bg-card p-6 text-card-foreground md:grid-cols-[1fr_2fr] [&_h2]:font-semibold [&>div>p]:mt-1 [&>div>p]:text-sm [&>div>p]:text-muted-foreground" aria-labelledby="create-heading">
      <div><h2 id="create-heading">Create a project</h2><p>Group related applications in one place.</p></div>
      <form onSubmit={createProject} className="flex flex-col gap-6">
        <FieldGroup>
          <Field><FieldLabel htmlFor="project-name">Project name</FieldLabel><Input id="project-name" name="name" required maxLength={80} autoFocus placeholder="e.g. Customer portal" disabled={saving} /></Field>
          <Field><FieldLabel htmlFor="description">Description <span className="text-muted-foreground">(optional)</span></FieldLabel><Input id="description" name="description" maxLength={500} placeholder="What will this project run?" disabled={saving} /><FieldDescription>You can add applications after deployment support is available.</FieldDescription></Field>
        </FieldGroup>
        {error ? <ErrorNotice message={error} /> : null}
        <div className="flex justify-end gap-2"><Button variant="outline" type="button" onClick={closeForm} disabled={saving}>Cancel</Button><Button type="submit" disabled={saving}>{saving ? 'Creating…' : 'Create project'}</Button></div>
      </form>
    </section> : null}
    {projects.data === null ? <ErrorNotice message={projects.error} /> : projects.data.length === 0 ? <section className="overflow-hidden rounded-xl border bg-card text-card-foreground [&>[data-slot=empty]]:min-h-72 [&_[data-slot=table-caption]]:p-4"><Empty><EmptyHeader><EmptyMedia variant="icon"><Folder /></EmptyMedia><EmptyTitle>Your first deployment starts here</EmptyTitle><EmptyDescription>Create a project to organise your applications. This workspace is ready for a fresh start.</EmptyDescription></EmptyHeader>{!creating ? <Button variant="outline" onClick={() => setCreating(true)}><Plus data-icon="inline-start" />Create your first project</Button> : null}</Empty></section> : <section className="overflow-hidden rounded-xl border bg-card text-card-foreground [&>[data-slot=empty]]:min-h-72 [&_[data-slot=table-caption]]:p-4" aria-label="Project list"><Table><TableCaption>Up to 100 most recent projects. Deployment support is coming next.</TableCaption><TableHeader><TableRow><TableHead>Project</TableHead><TableHead>Created</TableHead><TableHead>Stage</TableHead></TableRow></TableHeader><TableBody>{projects.data.map(project => <TableRow key={project.id}><TableCell><div className="flex items-center gap-3 [&_p]:mt-1 [&_p]:max-w-sm [&_p]:whitespace-normal [&_p]:text-xs [&_p]:text-muted-foreground"><span className="flex size-9 shrink-0 items-center justify-center rounded-lg bg-muted [&_svg]:size-4"><Folder aria-hidden="true" /></span><div><strong>{project.name}</strong><p>{project.description || 'No description'}</p></div></div></TableCell><TableCell><time dateTime={project.created_at}>{new Date(project.created_at).toLocaleDateString(undefined, { day: 'numeric', month: 'short', year: 'numeric' })}</time></TableCell><TableCell><Badge variant="outline">Created</Badge></TableCell></TableRow>)}</TableBody></Table></section>}
    <div className="flex max-w-xl items-start gap-3 text-sm text-muted-foreground [&>svg]:size-5 [&>svg]:shrink-0 [&_h2]:font-medium [&_h2]:text-foreground [&_p]:mt-1"><Layers3 aria-hidden="true" /><div><h2>Built for your own infrastructure</h2><p>Projects are the first step. Application deployment, domains and build logs are planned next.</p></div></div>
  </>
}

function ContainersPage() {
  const { containers } = containersRoute.useLoaderData()
  return <><div className="flex flex-wrap items-center justify-between gap-4 [&_h1]:text-2xl [&_h1]:font-semibold [&_h1]:tracking-tight [&_p]:mt-1 [&_p]:text-sm [&_p]:text-muted-foreground"><div><h1>Containers</h1><p>A live view of containers on your connected Docker server.</p></div><Badge variant="outline">Read only</Badge></div>
    {containers.data === null ? <ErrorNotice message={containers.error} /> : containers.data.length === 0 ? <section className="overflow-hidden rounded-xl border bg-card text-card-foreground [&>[data-slot=empty]]:min-h-72 [&_[data-slot=table-caption]]:p-4"><Empty><EmptyHeader><EmptyMedia variant="icon"><Box /></EmptyMedia><EmptyTitle>No containers found</EmptyTitle><EmptyDescription>Containers on this Docker server will appear here. Use Refresh to check again.</EmptyDescription></EmptyHeader></Empty></section> : <section className="overflow-hidden rounded-xl border bg-card text-card-foreground [&>[data-slot=empty]]:min-h-72 [&_[data-slot=table-caption]]:p-4"><Table><TableCaption>All containers on the connected Docker server, including services outside Ferrix.</TableCaption><TableHeader><TableRow><TableHead>Container</TableHead><TableHead>Image</TableHead><TableHead>State</TableHead><TableHead>Status</TableHead></TableRow></TableHeader><TableBody>{containers.data.map(container => <TableRow key={container.id}><TableCell><strong>{container.name || container.id.slice(0, 12)}</strong></TableCell><TableCell>{container.image}</TableCell><TableCell><Badge variant={container.state === 'running' ? 'secondary' : 'outline'}>{container.state || 'Unknown'}</Badge></TableCell><TableCell>{container.status}</TableCell></TableRow>)}</TableBody></Table></section>}
  </>
}

function Pending() {
  return <div className="flex flex-col gap-6 p-6" role="status" aria-label="Loading workspace"><Skeleton className="h-12 w-52" /><Skeleton className="h-56 w-full" /></div>
}

const rootRoute = createRootRoute({
  component: Shell,
  loader: async () => ({ status: await load<SystemStatus>('/status') }),
  pendingComponent: Pending,
  notFoundComponent: () => <div className="mx-auto flex w-full max-w-7xl flex-col gap-6 p-4 md:p-6"><h1>Page not found</h1><Link to="/">Back to projects</Link></div>,
  errorComponent: () => <div className="mx-auto flex w-full max-w-7xl flex-col gap-6 p-4 md:p-6"><ErrorNotice message="The workspace could not load. Refresh the page to try again." /></div>,
})
const projectsRoute = createRoute({ getParentRoute: () => rootRoute, path: '/', component: ProjectsPage, loader: async () => ({ projects: await load<Project[]>('/projects') }) })
const containersRoute = createRoute({ getParentRoute: () => rootRoute, path: '/containers', component: ContainersPage, loader: async () => ({ containers: await load<Container[]>('/containers') }) })
const router = createRouter({ routeTree: rootRoute.addChildren([projectsRoute, containersRoute]), defaultPendingMs: 150, defaultPendingMinMs: 200 })
declare module '@tanstack/react-router' { interface Register { router: typeof router } }

createRoot(document.getElementById('root')!).render(<StrictMode><RouterProvider router={router} /></StrictMode>)
