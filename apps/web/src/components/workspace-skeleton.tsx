import { Card, CardContent, CardHeader } from '@/components/ui/card'
import { Skeleton } from '@/components/ui/skeleton'

export function WorkspaceSkeleton({ kind = 'projects' }: { kind?: 'projects' | 'service' }) {
  return <div role="status" aria-label={kind === 'service' ? 'Loading service' : 'Loading projects'} className="flex min-w-0 flex-col gap-6">
    <span className="sr-only">{kind === 'service' ? 'Loading service…' : 'Loading projects…'}</span>
    <div aria-hidden="true" className="flex min-w-0 flex-col gap-6">
      <div className="flex flex-col gap-3"><Skeleton className="h-4 w-40 max-w-full" /><Skeleton className="h-8 w-64 max-w-full" /><Skeleton className="h-4 w-80 max-w-full" /></div>
      {kind === 'service' ? <>
        <div className="flex flex-wrap gap-2">{[0, 1, 2, 3].map(item => <Skeleton key={item} className="h-9 w-24" />)}</div>
        <Card><CardHeader><Skeleton className="h-6 w-40" /><Skeleton className="h-4 w-3/4" /></CardHeader><CardContent className="flex min-h-48 flex-col gap-4"><Skeleton className="h-9 w-full max-w-sm" /><Skeleton className="h-4 w-2/3" /><Skeleton className="h-4 w-1/2" /></CardContent></Card>
      </> : <>
        <div className="flex flex-wrap gap-3"><Skeleton className="h-9 min-w-0 flex-1 basis-full sm:basis-0" /><Skeleton className="h-9 w-36" /><Skeleton className="h-9 w-20" /></div>
        <div className="grid gap-4 md:grid-cols-2 xl:grid-cols-3">{[0, 1, 2].map(item => <Card key={item}><CardHeader><Skeleton className="h-6 w-2/3" /><Skeleton className="h-4 w-full" /><Skeleton className="h-4 w-4/5" /></CardHeader><CardContent className="flex flex-col gap-6"><Skeleton className="h-6 w-4/5" /><div className="flex justify-between gap-4"><Skeleton className="h-4 w-24" /><Skeleton className="h-4 w-16" /></div></CardContent></Card>)}</div>
      </>}
    </div>
  </div>
}
