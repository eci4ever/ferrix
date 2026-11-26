import { useEffect, useState } from 'react'
import { toast } from 'sonner'
import { request } from '@/lib/api'
import { Button } from '@/components/ui/button'
import { Input } from '@/components/ui/input'
import { Label } from '@/components/ui/label'
import { Checkbox } from '@/components/ui/checkbox'
import { Collapsible, CollapsibleTrigger, CollapsibleContent } from '@/components/ui/collapsible'
import { ChevronDown } from 'lucide-react'

type Policy = { enabled: boolean; cron: string; timezone: string; keep_last: number; next_run_at: string | null; last_run_at: string | null; last_message: string | null }
export function LogicalBackupPolicy({ serviceId, databaseId }: { serviceId: string; databaseId: string }) {
  const base = `/services/${serviceId}/databases/${databaseId}/backup-policy`
  const [policy, setPolicy] = useState<Policy | null>(null)
  const [error, setError] = useState('')
  const [busy, setBusy] = useState(false)
  const [enabled, setEnabled] = useState(false)
  const [version, setVersion] = useState(0)
  useEffect(() => {
    const abort = new AbortController()
    setPolicy(null); setError('')
    request<Policy>(base, { signal: abort.signal }).then(value => { if (!abort.signal.aborted) { setPolicy(value); setEnabled(value.enabled) } }).catch(err => { if (!abort.signal.aborted) setError(err.message) })
    return () => abort.abort()
  }, [base, version])
  useEffect(() => {
    const abort = new AbortController()
    const timer = window.setInterval(() => { if (!document.hidden) void request<Policy>(base, { signal: abort.signal }).then(p => { if (!abort.signal.aborted) setPolicy(p) }).catch(() => {}) }, 30_000)
    return () => { abort.abort(); clearInterval(timer) }
  }, [base])
  return <Collapsible className="border-t pt-3"><CollapsibleTrigger asChild><Button variant="ghost" size="sm">Automatic backups <ChevronDown aria-hidden="true" className="size-4" /></Button></CollapsibleTrigger><CollapsibleContent className="pt-3">
    <p className="mb-3 text-sm text-muted-foreground">Backs up this logical database only. Storage uses the shared MariaDB service’s saved destination. Databases due together run sequentially. Retention applies only to this database’s backups.</p>
    {error && <p role="alert" className="mb-3 text-sm text-destructive">{error}</p>}
    {!policy ? <Button variant="outline" size="sm" onClick={() => setVersion(v => v + 1)}>{error ? 'Retry settings' : 'Loading settings…'}</Button> : <form className="grid gap-4" onSubmit={async event => {
      event.preventDefault(); const data = new FormData(event.currentTarget); setBusy(true); setError('')
      try { await request(base, { method: 'POST', headers: { 'Content-Type': 'application/json' }, body: JSON.stringify({ enabled, cron: data.get('cron'), timezone: data.get('timezone'), keep_last: Number(data.get('keep_last')) }) }); setPolicy(await request<Policy>(base)); toast.success('Backup schedule saved') }
      catch (err) { const message = err instanceof Error ? err.message : 'Could not save schedule.'; setError(message); toast.error(message) } finally { setBusy(false) }
    }}><fieldset disabled={busy} className="grid gap-4">
      <div className="flex items-center gap-2"><Checkbox id={`logical-enabled-${databaseId}`} checked={enabled} onCheckedChange={value => setEnabled(value === true)} /><Label htmlFor={`logical-enabled-${databaseId}`}>Enable automatic backups for this database</Label></div>
      <div className="grid gap-4 md:grid-cols-3"><div className="grid gap-2"><Label htmlFor={`logical-cron-${databaseId}`}>Cron expression</Label><Input id={`logical-cron-${databaseId}`} name="cron" defaultValue={policy.cron} maxLength={128} required /><p className="text-sm text-muted-foreground">Five fields. 0 2 * * * runs daily at 02:00.</p></div><div className="grid gap-2"><Label htmlFor={`logical-zone-${databaseId}`}>Timezone</Label><Input id={`logical-zone-${databaseId}`} name="timezone" defaultValue={policy.timezone} required placeholder="Asia/Kuala_Lumpur" /></div><div className="grid gap-2"><Label htmlFor={`logical-keep-${databaseId}`}>Successful backups to keep</Label><Input id={`logical-keep-${databaseId}`} name="keep_last" type="number" min={1} max={365} defaultValue={policy.keep_last} required /></div></div>
      <Button className="justify-self-start" disabled={busy}>{busy ? 'Saving…' : 'Save schedule'}</Button>
    </fieldset><p role="status" className="text-sm text-muted-foreground">Next run: {policy.enabled && policy.next_run_at ? new Date(policy.next_run_at).toLocaleString() : 'Schedule disabled'}{policy.last_run_at ? ` · Last checked: ${new Date(policy.last_run_at).toLocaleString()}` : ''}{policy.last_message ? ` · ${policy.last_message}` : ''}</p></form>}
  </CollapsibleContent></Collapsible>
}
