import { useEffect, useState } from 'react'
import { Link, useRouter } from '@tanstack/react-router'
import { ArrowRight, ChevronDown, Folder, Info, Layers3, LayoutGrid, List, Search, Trash2, X } from 'lucide-react'
import { Button } from '@/components/ui/button'
import { Card, CardHeader, CardTitle, CardDescription, CardContent, CardAction, CardFooter } from '@/components/ui/card'
import { Input } from '@/components/ui/input'
import { Table, TableHeader, TableHead, TableBody, TableRow, TableCell, TableCaption } from '@/components/ui/table'
import { DropdownMenu, DropdownMenuTrigger, DropdownMenuContent, DropdownMenuItem } from '@/components/ui/dropdown-menu'
import { Tooltip, TooltipContent, TooltipTrigger } from '@/components/ui/tooltip'
import { Empty, EmptyHeader, EmptyMedia, EmptyTitle, EmptyDescription } from '@/components/ui/empty'
import type { Project } from '@/lib/api'
import { request } from '@/lib/api'
import { ProjectStatusSummary } from './project-status-summary'
import { DeleteProjectDialog } from './delete-project-dialog'
import { ActionNotice } from '@/components/action-notice'
import type { ProjectRuntime } from './project-status-summary'

const sorts = { newest: 'Newest first', oldest: 'Oldest first', name: 'Name (A–Z)' }
const date = (value: string) => new Date(value).toLocaleDateString(undefined, { day: 'numeric', month: 'short', year: 'numeric' })

export function ProjectList({ projects, canManage = false }: { projects: Project[]; canManage?: boolean }) {
  const router = useRouter()
  const [deleting, setDeleting] = useState<Project | null>(null)
  const [removed, setRemoved] = useState<string[]>([])
  const [notice, setNotice] = useState('')
  function deleted() { if (!deleting) return; setRemoved(ids => [...ids, deleting.id]); setNotice(`Project “${deleting.name}” deleted.`); setDeleting(null); void router.invalidate().catch(() => setNotice('Project deleted. Refresh to reload the project list.')) }
  function deleteButton(project: Project) { return canManage ? <Tooltip><TooltipTrigger asChild><Button type="button" variant="ghost" size="icon" className="relative z-10 shrink-0" aria-label={`Delete ${project.name}`} onClick={() => setDeleting(project)}><Trash2 /></Button></TooltipTrigger><TooltipContent>Delete project</TooltipContent></Tooltip> : null }
  const [runtime, setRuntime] = useState<Record<string, ProjectRuntime>>({})
  const [runtimeError, setRuntimeError] = useState(false)
  const [checkedAt, setCheckedAt] = useState<string | null>(null)
  const projectIds = projects.map(project => project.id).join(',')
  useEffect(() => {
    let stopped = false
    let timer: ReturnType<typeof setTimeout>
    let controller: AbortController | undefined
    setRuntime({}); setRuntimeError(false); setCheckedAt(null)
    async function poll() {
      if (stopped) return
      if (!document.hidden && projectIds) {
        controller = new AbortController()
        try {
          const result = await request<{ checked_at: string; projects: ProjectRuntime[] }>('/projects/service-status', { signal: AbortSignal.any([controller.signal, AbortSignal.timeout(8000)]) })
          if (!stopped) { setRuntime(Object.fromEntries(result.projects.map(row => [row.id, row]))); setCheckedAt(result.checked_at); setRuntimeError(false) }
        } catch { if (!stopped) setRuntimeError(true) }
      }
      if (!stopped) timer = setTimeout(poll, 10_000)
    }
    void poll()
    return () => { stopped = true; clearTimeout(timer); controller?.abort() }
  }, [projectIds])
  const [view, setView] = useState<'cards' | 'table'>(() => { try { return localStorage.getItem('ferrix-project-view') === 'table' ? 'table' : 'cards' } catch { return 'cards' } })
  const [filter, setFilter] = useState('')
  const [sort, setSort] = useState<keyof typeof sorts>('newest')
  function choose(value: 'cards' | 'table') { setView(value); try { localStorage.setItem('ferrix-project-view', value) } catch { /* Storage may be unavailable. */ } }
  const available = projects.filter(p => !removed.includes(p.id))
  const visible = available.filter(p => `${p.name} ${p.description}`.toLowerCase().includes(filter.toLowerCase())).sort((a, b) => sort === 'name' ? a.name.localeCompare(b.name) || a.id.localeCompare(b.id) : (sort === 'oldest' ? 1 : -1) * (Date.parse(a.created_at) - Date.parse(b.created_at) || a.id.localeCompare(b.id)))
  return <section aria-label="Project list" className="grid min-w-0 gap-5">
    <ActionNotice message={notice} onDismiss={() => setNotice('')} />
    {deleting && <DeleteProjectDialog key={deleting.id} project={deleting} onClose={() => setDeleting(null)} onDeleted={deleted} />}
    <div className="flex flex-wrap items-center gap-3">
      <div className="relative min-w-0 basis-full sm:flex-1 sm:basis-0"><Search aria-hidden="true" className="pointer-events-none absolute start-3 top-1/2 size-4 -translate-y-1/2 text-muted-foreground" /><Input aria-label="Filter projects" placeholder="Filter projects…" className="ps-9 pe-10" value={filter} onChange={event => setFilter(event.target.value)} />{filter ? <Button variant="ghost" size="icon" className="absolute end-0 top-0" aria-label="Clear project search" onClick={() => setFilter('')}><X /></Button> : null}</div>
      <DropdownMenu><DropdownMenuTrigger asChild><Button variant="outline">{sorts[sort]}<ChevronDown aria-hidden="true" /></Button></DropdownMenuTrigger><DropdownMenuContent align="end">{(Object.keys(sorts) as (keyof typeof sorts)[]).map(value => <DropdownMenuItem key={value} onSelect={() => setSort(value)}>{sorts[value]}</DropdownMenuItem>)}</DropdownMenuContent></DropdownMenu>
      <div role="group" aria-label="Project view" className="flex items-center gap-1 rounded-lg bg-muted/40">{([{ value: 'cards', label: 'Card view', icon: LayoutGrid }, { value: 'table', label: 'Table view', icon: List }] as const).map(({ value, label, icon: Icon }) => <Tooltip key={value}><TooltipTrigger asChild><Button variant={view === value ? 'secondary' : 'ghost'} size="icon" aria-label={label} aria-pressed={view === value} onClick={() => choose(value)}><Icon /></Button></TooltipTrigger><TooltipContent>{label}</TooltipContent></Tooltip>)}</div>
    </div>
    {!visible.length ? <Card><CardContent><Empty className="min-h-56"><EmptyHeader><EmptyMedia variant="icon"><Search /></EmptyMedia><EmptyTitle>{available.length ? 'No matching projects' : 'No projects yet'}</EmptyTitle><EmptyDescription>{available.length ? 'Try a different project name or description.' : 'Create a project to organize your applications.'}</EmptyDescription></EmptyHeader>{filter ? <Button variant="outline" onClick={() => setFilter('')}>Clear search</Button> : null}</Empty></CardContent></Card> : view === 'cards' ? <div className="grid gap-4 md:grid-cols-2 xl:grid-cols-3">{visible.map(project => <Card key={project.id} className="relative min-w-0 transition-colors duration-150 hover:bg-muted/20 motion-reduce:transition-none">
      <CardHeader className="min-w-0 gap-2"><CardTitle className="min-w-0"><h2 className="flex min-w-0 items-start gap-3"><span className="flex size-9 shrink-0 items-center justify-center rounded-lg bg-muted text-muted-foreground"><Folder aria-hidden="true" className="size-4" /></span><Link to="/projects/$projectId" params={{ projectId: project.id }} className="min-w-0 flex-1 self-center break-words after:absolute after:inset-0 after:rounded-xl focus-visible:outline-none focus-visible:after:ring-2 focus-visible:after:ring-ring">{project.name}</Link></h2></CardTitle><CardDescription className="min-h-10 break-words leading-relaxed">{project.description || 'No description added.'}</CardDescription>{canManage ? <CardAction>{deleteButton(project)}</CardAction> : null}</CardHeader>
      <CardContent className="mt-auto"><ProjectStatusSummary summary={runtime[project.id]} failed={runtimeError} /></CardContent>
      <CardFooter className="flex-wrap justify-between gap-3 text-xs text-muted-foreground"><span>Created <time dateTime={project.created_at}>{date(project.created_at)}</time></span><span className="inline-flex items-center gap-2 font-medium tabular-nums text-foreground"><Layers3 aria-hidden="true" className="size-3.5" />{project.service_count} {project.service_count === 1 ? 'service' : 'services'}<ArrowRight aria-hidden="true" className="size-3.5 text-muted-foreground" /></span></CardFooter>
    </Card>)}</div> : <div className="overflow-hidden rounded-xl border bg-card"><Table><TableCaption>Up to 100 most recent projects. Open a project to manage environments and applications.</TableCaption><TableHeader><TableRow><TableHead>Project</TableHead><TableHead>Created</TableHead><TableHead className="text-right">Services</TableHead>{canManage && <TableHead>Actions</TableHead>}</TableRow></TableHeader><TableBody>{visible.map(project => <TableRow key={project.id}><TableCell><Link to="/projects/$projectId" params={{ projectId: project.id }} className="font-semibold hover:underline">{project.name}</Link><p className="max-w-sm whitespace-normal break-words text-xs text-muted-foreground">{project.description || 'No description'}</p></TableCell><TableCell><time dateTime={project.created_at}>{date(project.created_at)}</time></TableCell><TableCell className="text-right tabular-nums">{project.service_count}</TableCell>{canManage && <TableCell>{deleteButton(project)}</TableCell>}</TableRow>)}</TableBody></Table></div>}
    <div className="flex flex-wrap items-center justify-between gap-x-6 gap-y-2 text-xs text-muted-foreground"><p role="status">Showing {visible.length} of {available.length} projects · up to 100 most recent</p>
      {view === 'cards' && projectIds ? <div className="flex items-center gap-2"><span>{runtimeError ? 'Status unavailable · retrying automatically' : checkedAt ? `Updated ${new Date(checkedAt).toLocaleTimeString()} · every 10 seconds` : 'Checking runtime status…'}</span><Tooltip><TooltipTrigger asChild><Button variant="ghost" size="icon" aria-label="About project runtime status"><Info /></Button></TooltipTrigger><TooltipContent className="max-w-xs">Failed includes unhealthy or missing deployments. Other includes undeployed, starting and restarting services.</TooltipContent></Tooltip></div> : null}
    </div>
  </section>
}
