import { cn } from '@/lib/utils'

export function serviceState(state?: string, health?: string) {
  return state === 'running' && health === 'unhealthy' ? 'unhealthy' : state === 'running' && health === 'starting' ? 'starting' : state ?? 'checking'
}
export function ServiceStatus({ state, className, label = false }: { state?: string; className?: string; label?: boolean }) {
  const value = state ?? 'checking'
  const description = value.replaceAll('_', ' ')
  return <span className={cn('inline-flex items-center gap-2 text-sm', className)} title={`Service status: ${description}`}>
    <span aria-hidden="true" className={cn('size-3 shrink-0 rounded-full', value === 'running' ? 'bg-status-success' : ['unhealthy', 'dead', 'missing'].includes(value) ? 'bg-destructive' : ['restarting', 'partially_running', 'starting'].includes(value) ? 'bg-status-warning' : 'bg-status-neutral')} />
    <span className={label ? 'capitalize' : 'sr-only'}>Service status: {description}</span>
  </span>
}
