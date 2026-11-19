import { useState } from 'react'
import { Link } from '@tanstack/react-router'
import { Folder, LayoutGrid, List, Search } from 'lucide-react'
import { Button } from '@/components/ui/button'
import { Card, CardHeader, CardTitle, CardDescription, CardContent } from '@/components/ui/card'
import { Input } from '@/components/ui/input'
import { Table, TableHeader, TableHead, TableBody, TableRow, TableCell, TableCaption } from '@/components/ui/table'
import { DropdownMenu, DropdownMenuTrigger, DropdownMenuContent, DropdownMenuItem } from '@/components/ui/dropdown-menu'
import type { Project } from '@/lib/api'

const sorts = { newest: 'Newest first', oldest: 'Oldest first', name: 'Name (A–Z)' }
const date = (value: string) => new Date(value).toLocaleDateString(undefined, { day: 'numeric', month: 'short', year: 'numeric' })

export function ProjectList({ projects }: { projects: Project[] }) {
  const [view, setView] = useState<'cards' | 'table'>(() => { try { return localStorage.getItem('ferrix-project-view') === 'table' ? 'table' : 'cards' } catch { return 'cards' } })
  const [filter, setFilter] = useState('')
  const [sort, setSort] = useState<keyof typeof sorts>('newest')
  function choose(value: 'cards' | 'table') { setView(value); try { localStorage.setItem('ferrix-project-view', value) } catch { /* Storage may be unavailable. */ } }
  const visible = projects.filter(p => `${p.name} ${p.description}`.toLowerCase().includes(filter.toLowerCase())).sort((a, b) => sort === 'name' ? a.name.localeCompare(b.name) || a.id.localeCompare(b.id) : (sort === 'oldest' ? 1 : -1) * (Date.parse(a.created_at) - Date.parse(b.created_at) || a.id.localeCompare(b.id)))
  return <section aria-label="Project list" className="grid min-w-0 gap-5">
    <div className="flex flex-wrap items-center gap-3">
      <div className="relative min-w-0 basis-full sm:flex-1 sm:basis-0"><Search aria-hidden="true" className="absolute left-3 top-1/2 size-4 -translate-y-1/2 text-muted-foreground" /><Input aria-label="Filter projects" placeholder="Filter projects…" className="pl-9" value={filter} onChange={event => setFilter(event.target.value)} /></div>
      <DropdownMenu><DropdownMenuTrigger asChild><Button variant="outline">{sorts[sort]}</Button></DropdownMenuTrigger><DropdownMenuContent align="end">{(Object.keys(sorts) as (keyof typeof sorts)[]).map(value => <DropdownMenuItem key={value} onSelect={() => setSort(value)}>{sorts[value]}</DropdownMenuItem>)}</DropdownMenuContent></DropdownMenu>
      <div role="group" aria-label="Project view" className="flex gap-1"><Button variant={view === 'cards' ? 'secondary' : 'outline'} size="icon" aria-label="Card view" aria-pressed={view === 'cards'} onClick={() => choose('cards')}><LayoutGrid /></Button><Button variant={view === 'table' ? 'secondary' : 'outline'} size="icon" aria-label="Table view" aria-pressed={view === 'table'} onClick={() => choose('table')}><List /></Button></div>
    </div>
    {!visible.length ? <Card><CardContent className="py-10 text-center text-sm text-muted-foreground">No matching projects. Try a different search.</CardContent></Card> : view === 'cards' ? <div className="grid gap-4 md:grid-cols-2 xl:grid-cols-3">{visible.map(project => <Card key={project.id} className="relative min-w-0 transition-colors hover:bg-muted/30">
      <CardHeader><CardTitle><h2 className="flex min-w-0 items-start gap-3"><Folder aria-hidden="true" className="mt-0.5 size-5 shrink-0 text-muted-foreground" /><Link to="/projects/$projectId" params={{ projectId: project.id }} className="break-words after:absolute after:inset-0 after:rounded-xl focus-visible:outline-none focus-visible:after:ring-2 focus-visible:after:ring-ring">{project.name}</Link></h2></CardTitle><CardDescription className="min-h-12 break-words">{project.description || 'No description'}</CardDescription></CardHeader>
      <CardContent className="mt-auto flex flex-wrap items-center justify-between gap-3 text-xs text-muted-foreground"><span>Created <time dateTime={project.created_at}>{date(project.created_at)}</time></span><span className="font-medium text-foreground">{project.service_count} {project.service_count === 1 ? 'service' : 'services'}</span></CardContent>
    </Card>)}</div> : <div className="overflow-hidden rounded-xl border bg-card"><Table><TableCaption>Up to 100 most recent projects. Open a project to manage environments and applications.</TableCaption><TableHeader><TableRow><TableHead>Project</TableHead><TableHead>Created</TableHead></TableRow></TableHeader><TableBody>{visible.map(project => <TableRow key={project.id}><TableCell><Link to="/projects/$projectId" params={{ projectId: project.id }} className="font-semibold hover:underline">{project.name}</Link><p className="max-w-sm whitespace-normal break-words text-xs text-muted-foreground">{project.description || 'No description'}</p></TableCell><TableCell><time dateTime={project.created_at}>{date(project.created_at)}</time></TableCell></TableRow>)}</TableBody></Table></div>}
    {view === 'cards' && <p className="text-xs text-muted-foreground">Showing {visible.length} of {projects.length} loaded projects · up to 100 most recent.</p>}
  </section>
}
