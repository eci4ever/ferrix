import { FormDialog } from '@/components/form-dialog'
import { DialogFooter } from '@/components/ui/dialog'
import { ProfilePage } from './profile'
import type { Profile } from './profile'
import { createContext, useContext, StrictMode, useEffect, useRef, useState } from 'react'
import type { FormEvent } from 'react'
import { createRoot } from 'react-dom/client'
import { createRootRoute, createRoute, createRouter, RouterProvider, Outlet, Link, Navigate, useRouter, useRouterState } from '@tanstack/react-router'
import { Layers3, ArrowRight, RefreshCw, Plus, Folder, CheckCircle2, CircleAlert } from 'lucide-react'
import { Button } from '@/components/ui/button'
import { Badge } from '@/components/ui/badge'
import { Input } from '@/components/ui/input'
import { Field, FieldGroup, FieldLabel, FieldDescription } from '@/components/ui/field'
import { DataTable } from '@/components/data-table'
import { Alert, AlertTitle, AlertDescription } from '@/components/ui/alert'
import { Empty, EmptyHeader, EmptyMedia, EmptyTitle, EmptyDescription } from '@/components/ui/empty'
import { Skeleton } from '@/components/ui/skeleton'
import { ActionNotice } from '@/components/action-notice'
import { load, request } from '@/lib/api'
import type { Project, SystemStatus, HomeSummary } from '@/lib/api'
import { LoginPage, SecurityPage } from './auth'
import { TeamPage, AdminPage } from './user-management'
import { ApiKeysPage } from './api-keys'
import type { Identity, Security, Session, Audit, Organization, Member, Invitation } from './auth'
import { Notifications } from './notifications'
import { ServerClock } from '@/components/server-clock'
import { WorkspaceSkeleton } from '@/components/workspace-skeleton'
import { WorkspaceCommand } from '@/components/workspace-command'
import { AppSidebar } from '@/components/app-sidebar'
import { SidebarProvider, SidebarInset, SidebarTrigger } from '@/components/ui/sidebar'
import { Tooltip, TooltipContent, TooltipProvider, TooltipTrigger } from '@/components/ui/tooltip'
import { Breadcrumb, BreadcrumbList, BreadcrumbItem, BreadcrumbLink, BreadcrumbPage, BreadcrumbSeparator } from '@/components/ui/breadcrumb'
import { Separator } from '@/components/ui/separator'
import { Card, CardHeader, CardTitle, CardDescription, CardAction, CardContent } from '@/components/ui/card'
import { LandingPage } from './landing'
import { ProjectServices, ServiceWorkspace } from './project-services'
import { MonitoringPage } from './monitoring'
import { PlatformOrganizations } from './platform-organizations'
import { PlatformSettings } from './platform-settings'
import { DockerPage } from './docker'
import { ProjectList } from './project-list'
import './index.css'
import { Toaster } from '@/components/ui/sonner'

function ErrorNotice({ message }: { message: string }) {
  return <Alert variant="destructive"><CircleAlert /><AlertTitle>Something needs attention</AlertTitle><AlertDescription>{message}</AlertDescription></Alert>
}

const WorkspaceRefresh = createContext(0)
const WorkspaceReload = createContext<() => Promise<void>>(async () => {})

function Shell() {
  const { status, account, setup, organizations } = rootRoute.useLoaderData()
  const router = useRouter()
  const pathname = useRouterState({ select: state => state.location.pathname })
  const [refreshing, setRefreshing] = useState(false)
  const [refreshVersion, setRefreshVersion] = useState(0)
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
    setRefreshVersion(version => version + 1)
    try { await router.invalidate({ sync: true }) }
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
  if (pathname === '/') return <Outlet />
  if (pathname === '/login' || pathname === '/signup') {
    if (account.data) return <Navigate to="/dashboard" replace />
    if (setup.data === null) return <main className="mx-auto max-w-md p-6"><ErrorNotice message={setup.error} /><Button onClick={refresh} disabled={refreshing}>Retry</Button></main>
    return <LoginPage key={pathname} setupRequired={pathname === '/signup' && setup.data.setup_required} initialMode={pathname === '/signup' ? 'signup' : 'login'} refresh={() => router.invalidate()} />
  }
  if (!account.data) return <Navigate to="/login" replace />
  const pageNames: Record<string, string> = { '/dashboard': 'Home', '/monitoring': 'Monitoring', '/projects': 'Projects', '/containers': 'Docker', '/profile': 'Profile', '/security': 'Security', '/api-keys': 'API keys', '/organization': 'Organization', '/admin': 'Users', '/platform/settings': 'Platform Settings', '/organization/members': 'Members', '/organization/invitations': 'Invitations', '/organization/settings': 'Overview', '/platform/organizations': 'Organizations' }
  return <TooltipProvider><SidebarProvider>
    <a href="#main" className="sr-only focus:not-sr-only focus:absolute focus:bg-background focus:p-4">Skip to content</a>
    <AppSidebar identity={account.data} organizations={organizations.data ?? []} theme={theme} busy={accountBusy} onSwitch={id => accountAction('organizations/switch', { organization_id: id })} onSignOut={() => accountAction('sign-out')} onToggleTheme={() => setTheme(theme === 'dark' ? 'light' : 'dark')} version={status.data?.version} />
    <SidebarInset id="main" tabIndex={-1}>
      <header className="flex min-h-16 shrink-0 flex-wrap items-center justify-between gap-2 px-4 py-2 sm:flex-nowrap transition-[width,height] ease-linear group-has-data-[collapsible=icon]/sidebar-wrapper:h-12 motion-reduce:transition-none">
        <div className="flex min-w-0 max-w-full items-center gap-2"><SidebarTrigger className="-ml-1" /><Separator orientation="vertical" className="mr-2 data-vertical:h-4 data-vertical:self-center" /><Breadcrumb><BreadcrumbList className="min-w-0 flex-wrap"><BreadcrumbItem className="hidden md:block"><BreadcrumbLink asChild><Link to="/organization">{account.data.organization_name}</Link></BreadcrumbLink></BreadcrumbItem><BreadcrumbSeparator className="hidden md:block" /><BreadcrumbItem><BreadcrumbPage className="max-w-40 truncate sm:max-w-none">{pageNames[pathname] ?? (pathname.startsWith('/projects/') ? 'Projects' : 'Page not found')}</BreadcrumbPage></BreadcrumbItem></BreadcrumbList></Breadcrumb></div>
        <div className="ml-auto flex shrink-0 items-center gap-2"><WorkspaceCommand key={`command:${account.data.organization_id}`} identity={account.data} /><ServerClock refreshVersion={refreshVersion} /><Notifications key={account.data.organization_id} /><Tooltip><TooltipTrigger asChild><Button variant="outline" size="icon" onClick={refresh} disabled={refreshing} aria-label="Refresh workspace" aria-busy={refreshing}><RefreshCw aria-hidden="true" className={refreshing ? 'animate-spin motion-reduce:animate-none' : undefined} /></Button></TooltipTrigger><TooltipContent side="bottom">{refreshing ? 'Refreshing workspace data…' : 'Refresh current page data'}</TooltipContent></Tooltip></div>
      </header>
      <div className="flex min-w-0 flex-1 flex-col gap-6 p-4 pt-2 md:p-6 md:pt-2">
        {refreshError ? <ErrorNotice message={refreshError} /> : null}
        {organizations.error ? <ErrorNotice message={organizations.error} /> : null}
        <WorkspaceRefresh.Provider value={refreshVersion}><WorkspaceReload.Provider value={refresh}><Outlet /></WorkspaceReload.Provider></WorkspaceRefresh.Provider>
      </div>
    </SidebarInset>
  </SidebarProvider></TooltipProvider>
}

function HomePage() {
  const { projects, summary } = homeRoute.useLoaderData()
  const totals: HomeSummary | null = summary.data
  const { account, status } = rootRoute.useLoaderData()
  if (!account.data) return null
  const recent: Project[] = projects.data ?? []
  return <div className="flex flex-1 flex-col gap-6">
    <div className="flex flex-wrap items-start justify-between gap-4"><div className="min-w-0"><h1 className="break-words text-2xl font-semibold tracking-tight md:text-3xl">Welcome back, {account.data.name}</h1><p className="mt-1 text-sm text-muted-foreground">Your deployment workspace at a glance.</p></div><Button asChild variant="secondary"><Link to="/projects">Go to projects<ArrowRight data-icon="inline-end" /></Link></Button></div>
    <div className="flex flex-col gap-6">
      <div className="grid gap-4 sm:grid-cols-2 xl:grid-cols-4">
        <Card><CardHeader className="min-h-16"><CardTitle>Projects</CardTitle><CardDescription>In this organization</CardDescription></CardHeader><CardContent><p className="text-3xl font-semibold tabular-nums">{totals?.projects ?? 'Unavailable'}</p><p className="mt-2 text-xs text-muted-foreground">{totals ? `${totals.projects_new_7d} created in the last 7 days` : 'Summary could not be loaded'}</p></CardContent></Card>
        <Card><CardHeader className="min-h-16"><CardTitle>Services</CardTitle><CardDescription>Across all projects</CardDescription></CardHeader><CardContent><p className="text-3xl font-semibold tabular-nums">{totals?.services ?? 'Unavailable'}</p><p className="mt-2 text-xs text-muted-foreground">{totals ? `${totals.applications} applications · ${totals.databases} databases` : 'Summary could not be loaded'}</p></CardContent></Card>
        <Card><CardHeader className="min-h-16"><CardTitle>Deployments</CardTitle><CardDescription>Last 24 hours</CardDescription></CardHeader><CardContent><p className="text-3xl font-semibold tabular-nums">{totals?.deployments_24h ?? 'Unavailable'}</p><p className="mt-2 text-xs text-muted-foreground">{totals ? `${totals.succeeded_24h} succeeded · ${totals.failed_24h} failed · ${totals.in_progress_24h} queued or running` : 'Summary could not be loaded'}</p></CardContent></Card>
        <Card><CardHeader className="min-h-16"><CardTitle>Needs attention</CardTitle><CardDescription>Failed latest deployments</CardDescription></CardHeader><CardContent><p className="text-3xl font-semibold tabular-nums">{totals?.needs_attention ?? 'Unavailable'}</p>{totals ? totals.attention.length ? <><ul className="mt-2 space-y-1 text-sm">{totals.attention.map(service => <li key={service.id}><Link className="break-words underline underline-offset-4" to="/projects/$projectId/services/$serviceId" params={{ projectId: service.project_id, serviceId: service.id }}>{service.name}</Link></li>)}</ul>{totals.needs_attention > totals.attention.length ? <p className="mt-2 text-xs text-muted-foreground">Showing 5 of {totals.needs_attention} services. Open Projects to view all.</p> : null}</> : <p className="mt-2 text-xs text-muted-foreground">No failed latest deployments</p> : <p className="mt-2 text-xs text-muted-foreground">Summary could not be loaded</p>}</CardContent></Card>
      </div>
      {summary.error ? <ErrorNotice message={`${summary.error} Use Refresh to retry.`} /> : null}
      {status.data ? <div className="flex flex-wrap gap-4 text-sm"><Connection name="PostgreSQL" connected={status.data.database} /><Connection name="Docker" connected={status.data.docker} /></div> : null}
      {status.error ? <ErrorNotice message={status.error} /> : null}
      <Card><CardHeader><CardTitle>Recent projects</CardTitle><CardDescription>Your most recently created projects</CardDescription><CardAction><Button asChild variant="ghost"><Link to="/projects">View all<ArrowRight data-icon="inline-end" /></Link></Button></CardAction></CardHeader><CardContent>{projects.error ? <ErrorNotice message={projects.error} /> : recent.length ? <DataTable label="Recent projects" searchable={false} paginated={false} data={recent.slice(0, 5)} columns={[
        { id: 'name', label: 'Project', value: project => project.name, cell: project => <div className="min-w-48 max-w-sm whitespace-normal"><Link className="break-words font-medium hover:underline" to="/projects/$projectId" params={{ projectId: project.id }}>{project.name}</Link><p className="break-words text-xs text-muted-foreground">{project.description || 'No description'}</p></div> },
        { id: 'created', label: 'Created', value: project => project.created_at, cell: project => <time dateTime={project.created_at}>{new Date(project.created_at).toLocaleDateString()}</time> },
        { id: 'services', label: 'Services', value: project => project.service_count, numeric: true },
      ]} /> : <Empty><EmptyHeader><EmptyMedia variant="icon"><Folder /></EmptyMedia><EmptyTitle>No projects yet</EmptyTitle><EmptyDescription>Create your first project to start organizing your applications.</EmptyDescription></EmptyHeader><Button asChild variant="outline"><Link to="/projects">Go to projects</Link></Button></Empty>}</CardContent></Card>
    </div>
  </div>
}

function Connection({ name, connected }: { name: string; connected: boolean }) {
  return <Badge variant={connected ? 'secondary' : 'outline'}>{connected ? <CheckCircle2 data-icon="inline-start" /> : <CircleAlert data-icon="inline-start" />}{name}: {connected ? 'Connected' : 'Unavailable'}</Badge>
}

function ProjectsPage() {
  const { account } = rootRoute.useLoaderData()
  const { projects } = projectsRoute.useLoaderData()
  const router = useRouter()
  const [creating, setCreating] = useState(false)
  const [saving, setSaving] = useState(false)
  const [error, setError] = useState('')
  const [notice, setNotice] = useState('')
  const createButton = useRef<HTMLButtonElement>(null)
  const createOrigin = useRef<HTMLButtonElement | null>(null)
  const canManage = account.data?.role === 'owner' || account.data?.role === 'admin'

  function closeForm() { setCreating(false); setError('') }
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
    <div className="flex flex-wrap items-center justify-between gap-4 [&_h1]:text-2xl [&_h1]:font-semibold [&_h1]:tracking-tight [&_p]:mt-1 [&_p]:text-sm [&_p]:text-muted-foreground"><div><h1>Projects</h1><p>A home for the applications you deploy.</p></div>{canManage ? <Button ref={createButton} size="lg" onClick={event => { createOrigin.current = event.currentTarget; setCreating(true); setNotice(''); setError('') }} aria-haspopup="dialog"><Plus data-icon="inline-start" />New project</Button> : null}</div>
    <ActionNotice message={notice} onDismiss={() => setNotice('')} />
    <FormDialog open={creating} onOpenChange={open => { if (!open) closeForm() }} title="Create a project" description="Group related applications in one place." busy={saving}
      onCloseAutoFocus={event => { event.preventDefault(); (createOrigin.current?.isConnected ? createOrigin.current : createButton.current)?.focus() }}>
      <form onSubmit={createProject} className="flex flex-col gap-6">
        <FieldGroup>
          <Field><FieldLabel htmlFor="project-name">Project name</FieldLabel><Input id="project-name" name="name" required maxLength={80} autoFocus placeholder="e.g. Customer portal" disabled={saving} /></Field>
          <Field><FieldLabel htmlFor="description">Description <span className="text-muted-foreground">(optional)</span></FieldLabel><Input id="description" name="description" maxLength={500} placeholder="What will this project run?" disabled={saving} /><FieldDescription>Each project starts with a production environment.</FieldDescription></Field>
        </FieldGroup>
        {error ? <ErrorNotice message={error} /> : null}
        <DialogFooter><Button variant="outline" type="button" onClick={closeForm} disabled={saving}>Cancel</Button><Button type="submit" disabled={saving}>{saving ? 'Creating…' : 'Create project'}</Button></DialogFooter>
      </form>
    </FormDialog>
    {projects.data === null ? <ErrorNotice message={projects.error} /> : projects.data.length === 0 ? <section className="overflow-hidden rounded-xl border bg-card text-card-foreground [&>[data-slot=empty]]:min-h-72 [&_[data-slot=table-caption]]:p-4"><Empty><EmptyHeader><EmptyMedia variant="icon"><Folder /></EmptyMedia><EmptyTitle>Your first deployment starts here</EmptyTitle><EmptyDescription>Create a project to organise your applications. This workspace is ready for a fresh start.</EmptyDescription></EmptyHeader>{canManage ? <Button variant="outline" aria-haspopup="dialog" onClick={event => { createOrigin.current = event.currentTarget; setCreating(true); setError(''); setNotice('') }}><Plus data-icon="inline-start" />Create your first project</Button> : null}</Empty></section> : <ProjectList projects={projects.data} canManage={account.data?.role === 'owner' || account.data?.role === 'admin'} />}
    <div className="flex max-w-xl items-start gap-3 text-sm text-muted-foreground [&>svg]:size-5 [&>svg]:shrink-0 [&_h2]:font-medium [&_h2]:text-foreground [&_p]:mt-1"><Layers3 aria-hidden="true" /><div><h2>Built for your own infrastructure</h2><p>Create an environment, add an application and deploy its Docker image.</p></div></div>
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
const landingRoute = createRoute({ getParentRoute: () => rootRoute, path: '/', component: function Landing() { const { account } = rootRoute.useLoaderData(); return <LandingPage signedIn={!!account.data} /> } })
const loginRoute = createRoute({ getParentRoute: () => rootRoute, path: '/login' })
const signupRoute = createRoute({ getParentRoute: () => rootRoute, path: '/signup' })
const homeRoute = createRoute({ getParentRoute: () => rootRoute, path: '/dashboard', component: HomePage, loader: async () => { const [projects, summary] = await Promise.all([load<Project[]>('/projects'), load<HomeSummary>('/home-summary')]); return { projects, summary } } })
const projectsRoute = createRoute({ getParentRoute: () => rootRoute, path: '/projects', component: ProjectsPage, pendingComponent: WorkspaceSkeleton, loader: async () => ({ projects: await load<Project[]>('/projects') }) })
const projectRoute = createRoute({ getParentRoute: () => rootRoute, path: '/projects/$projectId', component: function ProjectDetail() { const { projectId } = projectRoute.useParams(); const { account } = rootRoute.useLoaderData(); return <ProjectServices key={`${account.data?.organization_id}:${projectId}`} projectId={projectId} canAdmin={account.data?.system_role === 'admin'} canManage={account.data?.role === 'owner' || account.data?.role === 'admin'} /> } })
const applicationRoute = createRoute({ getParentRoute: () => rootRoute, path: '/projects/$projectId/applications/$applicationId', component: function ApplicationDetail() { const { projectId, applicationId } = applicationRoute.useParams(); const { account } = rootRoute.useLoaderData(); return <ServiceWorkspace key={`${account.data?.organization_id}:${applicationId}`} projectId={projectId} serviceId={applicationId} canManage={account.data?.role === 'owner' || account.data?.role === 'admin'} /> } })
const serviceRoute = createRoute({ getParentRoute: () => rootRoute, path: '/projects/$projectId/services/$serviceId', component: function ServiceDetail() { const { projectId, serviceId } = serviceRoute.useParams(); const { account } = rootRoute.useLoaderData(); return <ServiceWorkspace key={`${account.data?.organization_id}:${serviceId}`} projectId={projectId} serviceId={serviceId} canManage={account.data?.role === 'owner' || account.data?.role === 'admin'} /> } })
const monitoringRoute = createRoute({ getParentRoute: () => rootRoute, path: '/monitoring', component: function Monitoring() { const { account } = rootRoute.useLoaderData(); return <MonitoringPage admin={account.data?.system_role === 'admin'} /> } })
const containersRoute = createRoute({ getParentRoute: () => rootRoute, path: '/containers', component: function Docker() { const { account } = rootRoute.useLoaderData(); return <DockerPage admin={account.data?.system_role === 'admin'} refreshVersion={useContext(WorkspaceRefresh)} /> } })
const profileRoute = createRoute({
  getParentRoute: () => rootRoute, path: '/profile',
  loader: () => load<Profile>('/auth/profile'),
  component: function ProfileSettings() {
    const result = profileRoute.useLoaderData()
    const router = useRouter()
    return result.data ? <ProfilePage profile={result.data} refresh={() => router.invalidate()} /> : <ErrorNotice message={result.error} />
  },
})
const securityRoute = createRoute({ getParentRoute: () => rootRoute, path: '/security', loader: async () => {
  const [sessions, security, audit] = await Promise.all([load<Session[]>('/auth/sessions'), load<Security>('/auth/security'), load<Audit[]>('/auth/audit')]); return { sessions, security, audit }
}, component: function AccountSecurity() { const { sessions, security, audit } = securityRoute.useLoaderData(); const router = useRouter(); return sessions.data && security.data ? <SecurityPage sessions={sessions.data} security={security.data} audit={audit.data ?? []} refresh={() => router.invalidate({ sync: true })} /> : <ErrorNotice message={sessions.error ?? security.error ?? 'Could not load security settings.'} /> } })
const teamRoute = createRoute({ getParentRoute: () => rootRoute, path: '/organization', loader: async () => {
  const [organizations, members, invitations] = await Promise.all([load<Organization[]>('/auth/organizations'), load<Member[]>('/auth/members'), load<Invitation[]>('/auth/invitations')]); return { organizations, members, invitations }
},
  component: function OrganizationSettings() {
    const { organizations, members, invitations } = teamRoute.useLoaderData()
    const { account } = rootRoute.useLoaderData()
    const router = useRouter()
    const refreshVersion = useContext(WorkspaceRefresh)
    if (!account.data || !organizations.data || !members.data) {
      return <ErrorNotice message="Could not load organization settings." />
    }
    return <TeamPage key={account.data.organization_id} identity={account.data}
      members={members.data} invitations={invitations.data ?? []}
      refreshVersion={refreshVersion} refresh={() => router.invalidate()} />
  },
})
const organizationLoader = async () => {
  const [members, invitations] = await Promise.all([load<Member[]>('/auth/members'), load<Invitation[]>('/auth/invitations')])
  return { members, invitations }
}
function OrganizationView({ members, invitations, view }: { members: Member[]; invitations: Invitation[]; view: 'settings' | 'team' | 'invitations' }) {
  const { account } = rootRoute.useLoaderData()
  const router = useRouter()
  const refreshVersion = useContext(WorkspaceRefresh)
  return account.data ? <TeamPage key={`${account.data.organization_id}:${view}`} view={view} identity={account.data} members={members} invitations={invitations} refreshVersion={refreshVersion} refresh={() => router.invalidate()} /> : null
}
const organizationMembersRoute = createRoute({ getParentRoute: () => rootRoute, path: '/organization/members', loader: organizationLoader, component: function OrganizationMembers() {
  const { members, invitations } = organizationMembersRoute.useLoaderData()
  return members.data ? <OrganizationView view="team" members={members.data} invitations={invitations.data ?? []} /> : <ErrorNotice message={members.error ?? 'Could not load members.'} />
} })
const organizationInvitationsRoute = createRoute({ getParentRoute: () => rootRoute, path: '/organization/invitations', component: () => <Navigate to="/organization/members" replace /> })
const organizationSettingsRoute = createRoute({ getParentRoute: () => rootRoute, path: '/organization/settings', loader: organizationLoader, component: function OrganizationConfiguration() {
  const { members, invitations } = organizationSettingsRoute.useLoaderData()
  return members.data ? <OrganizationView view="settings" members={members.data} invitations={invitations.data ?? []} /> : <ErrorNotice message={members.error ?? 'Could not load settings.'} />
} })
const platformOrganizationsRoute = createRoute({ getParentRoute: () => rootRoute, path: '/platform/organizations', component: function InstallationOrganizations() {
  const { account } = rootRoute.useLoaderData()
  const refreshVersion = useContext(WorkspaceRefresh)
  const refresh = useContext(WorkspaceReload)
  return <PlatformOrganizations admin={account.data?.system_role === 'admin'} activeOrganization={account.data?.organization_id ?? ''} refreshVersion={refreshVersion} refresh={refresh} />
} })
const platformSettingsRoute = createRoute({ getParentRoute: () => rootRoute, path: '/platform/settings', component: function InstallationSettings() {
  const { account } = rootRoute.useLoaderData()
  const refreshVersion = useContext(WorkspaceRefresh)
  const refresh = useContext(WorkspaceReload)
  return <PlatformSettings admin={account.data?.system_role === 'admin'} refreshVersion={refreshVersion} refresh={refresh} />
} })
const apiKeysRoute = createRoute({ getParentRoute: () => rootRoute, path: '/api-keys', component: function Keys() { const { account } = rootRoute.useLoaderData(); return account.data ? <ApiKeysPage key={account.data.organization_id} identity={account.data} /> : null } })
const adminRoute = createRoute({
  getParentRoute: () => rootRoute,
  path: '/admin',
  component: function UserAdministration() {
    const { account } = rootRoute.useLoaderData()
    const router = useRouter()
    const refreshVersion = useContext(WorkspaceRefresh)
    return account.data ? <AdminPage identity={account.data}
      refreshVersion={refreshVersion} refresh={() => router.invalidate()} /> : null
  },
})
const router = createRouter({ routeTree: rootRoute.addChildren([landingRoute, loginRoute, signupRoute, homeRoute, projectsRoute, projectRoute, applicationRoute, serviceRoute, monitoringRoute, containersRoute, profileRoute, securityRoute, teamRoute, organizationMembersRoute, organizationInvitationsRoute, organizationSettingsRoute, platformSettingsRoute, platformOrganizationsRoute, apiKeysRoute, adminRoute]), defaultPendingMs: 150, defaultPendingMinMs: 200 })
declare module '@tanstack/react-router' { interface Register { router: typeof router } }

createRoot(document.getElementById('root')!).render(<StrictMode><RouterProvider router={router} /><Toaster /></StrictMode>)
