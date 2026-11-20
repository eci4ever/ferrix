import { Badge } from '@/components/ui/badge'

export type ProjectRuntime = { id: string; running: number; stopped: number; failed: number; other: number; available: boolean }
export function ProjectStatusSummary({ summary, failed }: { summary?: ProjectRuntime; failed: boolean }) {
  if (failed || summary?.available === false) return <p className="text-xs text-muted-foreground">Runtime status unavailable</p>
  if (!summary) return <p className="text-xs text-muted-foreground">Checking runtime status…</p>
  return <div className="flex flex-wrap gap-2" aria-label="Service status summary">
    <Badge variant="outline"><span aria-hidden="true" className="size-2 rounded-full bg-status-success" />{summary.running} running</Badge>
    <Badge variant="outline"><span aria-hidden="true" className="size-2 rounded-full bg-status-neutral" />{summary.stopped} stopped</Badge>
    <Badge variant="outline"><span aria-hidden="true" className="size-2 rounded-full bg-destructive" />{summary.failed} failed</Badge>
    {summary.other > 0 && <Badge variant="outline">{summary.other} other</Badge>}
  </div>
}
