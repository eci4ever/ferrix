import { StrictMode, useEffect, useRef, useState } from 'react'
import type { FormEvent } from 'react'
import { createRoot } from 'react-dom/client'
import { createRootRoute, createRoute, createRouter, RouterProvider, Outlet, Link, useRouter, useRouterState } from '@tanstack/react-router'
import { Box, Layers3, ArrowRight, RefreshCw, Plus, Folder, CheckCircle2, CircleAlert } from 'lucide-react'
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
import { LoginPage, SecurityPage, TeamPage, AdminPage } from './auth'
import type { Identity, Security, Session, Audit, Organization, Member, Invitation, User } from './auth'
import { AppSidebar } from '@/components/app-sidebar'
import { SidebarProvider, SidebarInset, SidebarTrigger } from '@/components/ui/sidebar'
import { TooltipProvider } from '@/components/ui/tooltip'
import { Breadcrumb, BreadcrumbList, BreadcrumbItem, BreadcrumbLink, BreadcrumbPage, BreadcrumbSeparator } from '@/components/ui/breadcrumb'
import { Separator } from '@/components/ui/separator'
import { Card, CardHeader, CardTitle, CardDescription, CardAction, CardContent } from '@/components/ui/card'
import './index.css'

function ErrorNotice({ message }: { message: string }) {
  return <Alert variant="destructive"><CircleAlert /><AlertTitle>Something needs attention</AlertTitle><AlertDescription>{message}</AlertDescription></Alert>
}

function Shell() {
  const { status, account, setup, organizations } = rootRoute.useLoaderData()
  const router = useRouter()
  const pathname = useRouterState({ select: state => state.location.pathname })
  const [refreshing, setRefreshing] = useState(false)
  const [accountBusy, setAccountBusy] = useState(false)
  const [refreshError, setRefreshError] = useState('')
  const [theme, setTheme] = useState<'dark' | 'light'>(() => {
    try { return localStorage.getItem('ferrix-theme') === 'light' ? 'light' : 'dark' } catch { return 'dark' }
  })
  useEffect(() => {
    document.documentElement.classList.toggle('dark', theme === 'dark')
    document.documentElement.style.colorScheme = theme
    try { localStorage.setItem('ferrix-theme', theme) } catch { /* Storage can be unavailable in private contexts. */ }
  }, [theme])
  async function refresh() {
    setRefreshing(true); setRefreshError('')
    try { await router.invalidate() }
    catch { setRefreshError('Refresh failed. Please try again.') }
    finally { setRefreshing(false) }
  }
  async function accountAction(path: string, body = {}) {
    setAccountBusy(true); setRefreshError('')
    try {
      await request(`/auth/${path}`, { method: 'POST', headers: { 'Content-Type': 'application/json' }, body: JSON.stringify(body) })
      await router.invalidate()
    } catch (error) { setRefreshError(error instanceof Error ? error.message : 'Account operation failed.') }
    finally { setAccountBusy(false) }
  }
  if (!account.data) {
    if (setup.data === null) return <main className="mx-auto max-w-md p-6"><ErrorNotice message={setup.error} /><Button onClick={refresh} disabled={refreshing}>Retry</Button></main>
    return <LoginPage setupRequired={setup.data.setup_required} refresh={() => router.invalidate()} />
  }
  const pageNames: Record<string, string> = { '/': 'Home', '/projects': 'Projects', '/containers': 'Docker', '/security': 'Security', '/organization': 'Organization', '/admin': 'Users' }
  return <TooltipProvider><SidebarProvider>
    <a href="#main" className="sr-only focus:not-sr-only focus:absolute focus:bg-background focus:p-4">Skip to content</a>
    <AppSidebar identity={account.data} organizations={organizations.data ?? []} theme={theme} busy={accountBusy} onSwitch={id => accountAction('organizations/switch', { organization_id: id })} onSignOut={() => accountAction('sign-out')} onToggleTheme={() => setTheme(theme === 'dark' ? 'light' : 'dark')} version={status.data?.version} />
    <SidebarInset id="main" tabIndex={-1}>
      <header className="flex h-16 shrink-0 items-center justify-between gap-2 px-4 transition-[width,height] ease-linear group-has-data-[collapsible=icon]/sidebar-wrapper:h-12 motion-reduce:transition-none">
        <div className="flex min-w-0 items-center gap-2"><SidebarTrigger className="-ml-1" /><Separator orientation="vertical" className="mr-2 data-[orientation=vertical]:h-4" /><Breadcrumb><BreadcrumbList><BreadcrumbItem className="hidden md:block"><BreadcrumbLink asChild><Link to="/organization">{account.data.organization_name}</Link></BreadcrumbLink></BreadcrumbItem><BreadcrumbSeparator className="hidden md:block" /><BreadcrumbItem><BreadcrumbPage>{pageNames[pathname] ?? 'Page not found'}</BreadcrumbPage></BreadcrumbItem></BreadcrumbList></Breadcrumb></div>
        <Button variant="outline" onClick={refresh} disabled={refreshing} aria-label="Refresh workspace"><RefreshCw data-icon="inline-start" /><span className="hidden sm:inline">{refreshing ? 'Refreshing…' : 'Refresh'}</span></Button>
      </header>
      <div className="flex min-w-0 flex-1 flex-col gap-6 p-4 pt-2 md:p-6 md:pt-2">
        {refreshError ? <ErrorNotice message={refreshError} /> : null}
        {organizations.error ? <ErrorNotice message={organizations.error} /> : null}
        <Outlet />
      </div>
    </SidebarInset>
  </SidebarProvider></TooltipProvider>
}

function HomePage() {
  const { projects } = homeRoute.useLoaderData()
  const { account, status } = rootRoute.useLoaderData()
  if (!account.data) return null
  const recent: Project[] = projects.data ?? []
  return <Card className="flex-1">
    <CardHeader><CardTitle><h1 className="text-2xl font-semibold tracking-tight md:text-3xl">Welcome back, {account.data.name}</h1></CardTitle><CardDescription>Your deployment workspace at a glance.</CardDescription><CardAction><Button asChild variant="secondary"><Link to="/projects">Go to projects<ArrowRight data-icon="inline-end" /></Link></Button></CardAction></CardHeader>
    <CardContent className="flex flex-col gap-6">
      <div className="grid gap-4 sm:grid-cols-2 xl:grid-cols-4">
        <Card><CardHeader><CardTitle>Projects</CardTitle><CardDescription>In this organization</CardDescription></CardHeader><CardContent><p className="text-3xl font-semibold">{projects.error ? 'Unavailable' : recent.length === 100 ? '100+' : recent.length}</p><p className="mt-2 text-xs text-muted-foreground">{recent.length === 100 ? 'Showing the 100 most recent' : 'Ready to organize your applications'}</p></CardContent></Card>
        <Card><CardHeader><CardTitle>Organization</CardTitle><CardDescription>Current workspace</CardDescription></CardHeader><CardContent><p className="truncate font-semibold">{account.data.organization_name}</p><Badge variant="secondary" className="mt-3">{account.data.role}</Badge></CardContent></Card>
        <Card><CardHeader><CardTitle>Deployments</CardTitle><CardDescription>Deployment history</CardDescription></CardHeader><CardContent><p className="text-xl font-semibold">Not configured</p><p className="mt-2 text-xs text-muted-foreground">Deployment support is coming next</p></CardContent></Card>
        <Card><CardHeader><CardTitle>Status</CardTitle><CardDescription>Connected services</CardDescription></CardHeader><CardContent className="flex flex-col items-start gap-3">{status.data ? <><Connection name="PostgreSQL" connected={status.data.database} /><Connection name="Docker" connected={status.data.docker} /></> : <p className="text-sm text-muted-foreground">Unable to check connections</p>}</CardContent></Card>
      </div>
      {status.error ? <ErrorNotice message={status.error} /> : null}
      <Card><CardHeader><CardTitle>Recent projects</CardTitle><CardDescription>Your most recently created projects</CardDescription><CardAction><Button asChild variant="ghost"><Link to="/projects">View all<ArrowRight data-icon="inline-end" /></Link></Button></CardAction></CardHeader><CardContent>{projects.error ? <ErrorNotice message={projects.error} /> : recent.length ? <Table><TableHeader><TableRow><TableHead>Project</TableHead><TableHead>Created</TableHead><TableHead>Stage</TableHead></TableRow></TableHeader><TableBody>{recent.slice(0, 5).map(project => <TableRow key={project.id}><TableCell><p className="font-medium">{project.name}</p><p className="mt-1 max-w-sm truncate text-xs text-muted-foreground">{project.description || 'No description'}</p></TableCell><TableCell>{new Date(project.created_at).toLocaleDateString()}</TableCell><TableCell><Badge variant="outline">Created</Badge></TableCell></TableRow>)}</TableBody></Table> : <Empty><EmptyHeader><EmptyMedia variant="icon"><Folder /></EmptyMedia><EmptyTitle>No projects yet</EmptyTitle><EmptyDescription>Create your first project to start organizing your applications.</EmptyDescription></EmptyHeader><Button asChild variant="outline"><Link to="/projects">Go to projects</Link></Button></Empty>}</CardContent></Card>
    </CardContent>
  </Card>
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
    {projects.data === null ? <ErrorNotice message={projects.error} /> : projects.data.length === 0 ? <section className="overflow-hidden rounded-xl border bg-card text-card-foreground [&>[data-slot=empty]]:min-h-72 [&_[data-slot=table-caption]]:p-4"><Empty><EmptyHeader><EmptyMedia variant="icon"><Folder /></EmptyMedia><EmptyTitle>Your first deployment starts here</EmptyTitle><EmptyDescription>Create a project to organise your applications. This workspace is ready for a fresh start.</EmptyDescription></EmptyHeader>{!creating ? <Button variant="outline" onClick={() => setCreating(true)}><Plus data-icon="inline-start" />Create your first project</Button> : null}</Empty></section> : <section className="overflow-hidden rounded-xl border bg-card text-card-foreground [&>[data-slot=empty]]:min-h-72 [&_[data-slot=table-caption]]:p-4" aria-label="Project list"><Table><TableCaption>Up to 100 most recent projects. Deployment support is coming next.</TableCaption><TableHeader><TableRow><TableHead>Project</TableHead><TableHead>Created</TableHead><TableHead>Stage</TableHead></TableRow></TableHeader><TableBody>{projects.data.map((project: Project) => <TableRow key={project.id}><TableCell><div className="flex items-center gap-3 [&_p]:mt-1 [&_p]:max-w-sm [&_p]:whitespace-normal [&_p]:text-xs [&_p]:text-muted-foreground"><span className="flex size-9 shrink-0 items-center justify-center rounded-lg bg-muted [&_svg]:size-4"><Folder aria-hidden="true" /></span><div><strong>{project.name}</strong><p>{project.description || 'No description'}</p></div></div></TableCell><TableCell><time dateTime={project.created_at}>{new Date(project.created_at).toLocaleDateString(undefined, { day: 'numeric', month: 'short', year: 'numeric' })}</time></TableCell><TableCell><Badge variant="outline">Created</Badge></TableCell></TableRow>)}</TableBody></Table></section>}
    <div className="flex max-w-xl items-start gap-3 text-sm text-muted-foreground [&>svg]:size-5 [&>svg]:shrink-0 [&_h2]:font-medium [&_h2]:text-foreground [&_p]:mt-1"><Layers3 aria-hidden="true" /><div><h2>Built for your own infrastructure</h2><p>Projects are the first step. Application deployment, domains and build logs are planned next.</p></div></div>
  </>
}

function ContainersPage() {
  const { containers } = containersRoute.useLoaderData()
  return <><div className="flex flex-wrap items-center justify-between gap-4 [&_h1]:text-2xl [&_h1]:font-semibold [&_h1]:tracking-tight [&_p]:mt-1 [&_p]:text-sm [&_p]:text-muted-foreground"><div><h1>Containers</h1><p>A live view of containers on your connected Docker server.</p></div><Badge variant="outline">Read only</Badge></div>
    {containers.data === null ? <ErrorNotice message={containers.error} /> : containers.data.length === 0 ? <section className="overflow-hidden rounded-xl border bg-card text-card-foreground [&>[data-slot=empty]]:min-h-72 [&_[data-slot=table-caption]]:p-4"><Empty><EmptyHeader><EmptyMedia variant="icon"><Box /></EmptyMedia><EmptyTitle>No containers found</EmptyTitle><EmptyDescription>Containers on this Docker server will appear here. Use Refresh to check again.</EmptyDescription></EmptyHeader></Empty></section> : <section className="overflow-hidden rounded-xl border bg-card text-card-foreground [&>[data-slot=empty]]:min-h-72 [&_[data-slot=table-caption]]:p-4"><Table><TableCaption>All containers on the connected Docker server, including services outside Ferrix.</TableCaption><TableHeader><TableRow><TableHead>Container</TableHead><TableHead>Image</TableHead><TableHead>State</TableHead><TableHead>Status</TableHead></TableRow></TableHeader><TableBody>{containers.data.map((container: Container) => <TableRow key={container.id}><TableCell><strong>{container.name || container.id.slice(0, 12)}</strong></TableCell><TableCell>{container.image}</TableCell><TableCell><Badge variant={container.state === 'running' ? 'secondary' : 'outline'}>{container.state || 'Unknown'}</Badge></TableCell><TableCell>{container.status}</TableCell></TableRow>)}</TableBody></Table></section>}
  </>
}

function Pending() {
  return <div className="flex flex-col gap-6 p-6" role="status" aria-label="Loading workspace"><Skeleton className="h-12 w-52" /><Skeleton className="h-56 w-full" /></div>
}

const rootRoute = createRootRoute({
  component: Shell,
  loader: async () => { const [status, account, setup] = await Promise.all([load<SystemStatus>('/status'), load<Identity>('/auth/session'), load<{ setup_required: boolean }>('/auth/setup')]); const organizations = account.data ? await load<Organization[]>('/auth/organizations') : { data: [] as Organization[], error: null }; return { status, account, setup, organizations } },
  pendingComponent: Pending,
  notFoundComponent: () => <div className="mx-auto flex w-full max-w-7xl flex-col gap-6 p-4 md:p-6"><h1>Page not found</h1><Link to="/projects">Back to projects</Link></div>,
  errorComponent: () => <div className="mx-auto flex w-full max-w-7xl flex-col gap-6 p-4 md:p-6"><ErrorNotice message="The workspace could not load. Refresh the page to try again." /></div>,
})
const homeRoute = createRoute({ getParentRoute: () => rootRoute, path: '/', component: HomePage, loader: async () => ({ projects: await load<Project[]>('/projects') }) })
const projectsRoute = createRoute({ getParentRoute: () => rootRoute, path: '/projects', component: ProjectsPage, loader: async () => ({ projects: await load<Project[]>('/projects') }) })
const containersRoute = createRoute({ getParentRoute: () => rootRoute, path: '/containers', component: ContainersPage, loader: async () => ({ containers: await load<Container[]>('/containers') }) })
const securityRoute = createRoute({ getParentRoute: () => rootRoute, path: '/security', loader: async () => {
  const [sessions, security, audit] = await Promise.all([load<Session[]>('/auth/sessions'), load<Security>('/auth/security'), load<Audit[]>('/auth/audit')]); return { sessions, security, audit }
}, component: function AccountSecurity() { const { sessions, security, audit } = securityRoute.useLoaderData(); const router = useRouter(); return sessions.data && security.data ? <SecurityPage sessions={sessions.data} security={security.data} audit={audit.data ?? []} refresh={() => router.invalidate()} /> : <ErrorNotice message={sessions.error ?? security.error ?? 'Could not load security settings.'} /> } })
const teamRoute = createRoute({ getParentRoute: () => rootRoute, path: '/organization', loader: async () => {
  const [organizations, members, invitations] = await Promise.all([load<Organization[]>('/auth/organizations'), load<Member[]>('/auth/members'), load<Invitation[]>('/auth/invitations')]); return { organizations, members, invitations }
}, component: function OrganizationSettings() { const { organizations, members, invitations } = teamRoute.useLoaderData(); const { account } = rootRoute.useLoaderData(); const router = useRouter(); return account.data && organizations.data && members.data ? <TeamPage identity={account.data} organizations={organizations.data} members={members.data} invitations={invitations.data ?? []} refresh={() => router.invalidate()} /> : <ErrorNotice message="Could not load organization settings." /> } })
const adminRoute = createRoute({ getParentRoute: () => rootRoute, path: '/admin', loader: () => load<User[]>('/auth/admin/users'), component: function UserAdministration() { const users = adminRoute.useLoaderData(); const router = useRouter(); return users.data ? <AdminPage users={users.data} refresh={() => router.invalidate()} /> : <ErrorNotice message={users.error} /> } })
const router = createRouter({ routeTree: rootRoute.addChildren([homeRoute, projectsRoute, containersRoute, securityRoute, teamRoute, adminRoute]), defaultPendingMs: 150, defaultPendingMinMs: 200 })
declare module '@tanstack/react-router' { interface Register { router: typeof router } }

createRoot(document.getElementById('root')!).render(<StrictMode><RouterProvider router={router} /></StrictMode>)
