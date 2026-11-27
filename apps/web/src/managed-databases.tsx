import { LoadingButton } from '@/components/loading-button'
import { LogicalBackupPolicy } from './logical-backup-policy'
import { NativeSelect, NativeSelectOption } from '@/components/ui/native-select'
import { useCallback, useEffect, useState } from 'react'
import { toast } from 'sonner'
import { request, ApiRequestError } from '@/lib/api'
import { Button } from '@/components/ui/button'
import { Card, CardContent, CardHeader, CardTitle, CardDescription } from '@/components/ui/card'
import { Badge } from '@/components/ui/badge'
import { Input } from '@/components/ui/input'
import { Label } from '@/components/ui/label'
import { Textarea } from '@/components/ui/textarea'
import { Alert, AlertTitle, AlertDescription } from '@/components/ui/alert'
import { Dialog, DialogContent, DialogHeader, DialogTitle, DialogDescription, DialogFooter } from '@/components/ui/dialog'

type Database = { id: string; application_id: string | null; database_name: string; username: string; status: string; max_connections: number; message: string }
type Application = { id: string; name: string; kind: string }
type Preview = { application_id: string; before: string; after: string }
type Backup = { id: string; managed_database_id: string | null; operation: string; status: string; message: string; pruned_at: string | null; pruning: boolean; created_at: string; scheduled_for: string | null }
const post = (body: unknown = {}) => ({ method: 'POST', headers: { 'Content-Type': 'application/json' }, body: JSON.stringify(body), signal: AbortSignal.timeout(120_000) })

export function ManagedDatabases({ serviceId, environmentId }: { serviceId: string; environmentId: string }) {
  const [databases, setDatabases] = useState<Database[]>([])
  const [backups, setBackups] = useState<Backup[]>([])
  const [applications, setApplications] = useState<Application[]>([])
  const [target, setTarget] = useState('')
  const [error, setError] = useState('')
  const [busy, setBusy] = useState(false)
  const [confirmation, setConfirmation] = useState('')
  const [action, setAction] = useState<{ database: Database; preview?: Preview; revoke?: boolean; rotate?: boolean; remove?: boolean; backup?: string } | null>(null)
  const base = `/services/${serviceId}/databases`
  const load = useCallback(async () => {
    const [dbs, apps, jobs] = await Promise.all([request<Database[]>(base), request<Application[]>(`/environments/${environmentId}/services`), request<Backup[]>(`/services/${serviceId}/backups`)])
    setDatabases(dbs); setApplications(apps.filter(app => app.kind === 'application')); setBackups(jobs)
  }, [base, environmentId])
  const hasPendingBackup = backups.some(job => ['queued', 'running'].includes(job.status))
  useEffect(() => { let pending = false; const timer = window.setInterval(() => { if (document.hidden || pending) return; pending = true; void load().catch(() => {}).finally(() => { pending = false }) }, hasPendingBackup ? 3000 : 10_000); return () => clearInterval(timer) }, [hasPendingBackup, load])
  useEffect(() => { load().catch(err => setError(err.message)) }, [load])
  async function run(work: () => Promise<void>) {
    setBusy(true); setError('')
    try { await work(); await load() } catch (err) { setError(err instanceof Error ? err.message : 'Database operation failed.'); await load().catch(() => {}) } finally { setBusy(false) }
  }
  return <Card><CardHeader><CardTitle><h2>Application databases</h2></CardTitle><CardDescription>One private MariaDB service, a separate database and restricted SQL account for each application.</CardDescription></CardHeader><CardContent className="flex flex-col gap-4">
    <Alert><AlertTitle>Shared resources, separate credentials</AlertTitle><AlertDescription>Each account is limited to its database and 10 simultaneous connections. CPU, memory and disk I/O are shared. Removing a Ferrix member does not revoke a password already disclosed. Removing an application retains its database. The Backups tab covers the configured database only; manage each logical database’s backups and schedule below.</AlertDescription></Alert>
    {error && <p role="alert" className="text-sm text-destructive">{error}</p>}
    <form className="flex flex-wrap items-end gap-2" onSubmit={event => { event.preventDefault(); void run(async () => { await request(base, post({ application_id: target })); toast.success('Database provisioned') }) }}>
      <div className="grid min-w-60 gap-2"><Label htmlFor="managed-database-application">Application in this environment</Label><NativeSelect id="managed-database-application" value={target} onChange={event => setTarget(event.target.value)} required disabled={busy}><NativeSelectOption value="">Choose application</NativeSelectOption>{applications.map(app => <NativeSelectOption key={app.id} value={app.id}>{app.name}</NativeSelectOption>)}</NativeSelect></div><Button disabled={busy || !target}>Provision database</Button>
    </form>
    {!databases.length && <p className="text-sm text-muted-foreground">No managed application databases yet. Deploy MariaDB before provisioning.</p>}
    {databases.map(db => <div key={db.id} className="grid gap-3 rounded-lg border p-4"><div className="flex flex-wrap items-center gap-2"><span className="break-all font-mono text-sm">{db.database_name}</span><Badge variant="secondary">{db.status}</Badge></div><p className="text-sm text-muted-foreground">{applications.find(app => app.id === db.application_id)?.name ?? 'Application removed — database retained'} · {db.max_connections} connections</p><p className="text-sm">{db.message}</p><div className="flex flex-wrap gap-2">
      {db.status === 'failed' && <Button variant="outline" disabled={busy} onClick={() => void run(async () => { await request(`${base}/${db.id}/retry`, post()); toast.success('Database provisioned') })}>Retry provisioning</Button>}
      {db.status === 'ready' && db.application_id && <Button variant="outline" disabled={busy} onClick={() => void run(async () => { const preview = await request<Preview>(`${base}/${db.id}/environment`); setConfirmation(''); setAction({ database: db, preview }) })}>Preview connection settings</Button>}
      {db.status === 'ready' && <Button variant="outline" disabled={busy} onClick={() => { setConfirmation(''); setAction({ database: db, revoke: true }) }}>Revoke SQL account</Button>}
      {['ready', 'rotating'].includes(db.status) && <Button variant="outline" disabled={busy} onClick={() => { setConfirmation(''); setAction({ database: db, rotate: true }) }}>{db.status === 'rotating' ? 'Resume password rotation' : 'Rotate password'}</Button>}
      {db.status === 'ready' && <Button variant="outline" disabled={busy} onClick={() => void run(async () => { await request(`${base}/${db.id}/backups`, post()); toast.success('Logical database backup queued') })}>Back up this database</Button>}
      <Button variant="destructive" disabled={busy || db.status === 'rotating'} onClick={() => { setConfirmation(''); setAction({ database: db, remove: true }) }}>Delete database and account</Button>
    </div><LogicalBackupPolicy serviceId={serviceId} databaseId={db.id} />{backups.filter(job => job.managed_database_id === db.id).map(job => <div key={job.id} className="flex flex-wrap items-center gap-2 border-t pt-3 text-sm"><Badge variant="secondary">{job.operation} · {job.status}</Badge><span>{new Date(job.created_at).toLocaleString()}</span>{job.scheduled_for && <Badge variant="outline">Scheduled</Badge>}<p className="basis-full text-muted-foreground">{job.message}</p>{job.operation === 'backup' && job.status === 'succeeded' && !job.pruned_at && !job.pruning && db.status === 'ready' && <Button variant="outline" size="sm" disabled={busy} onClick={() => { setConfirmation(''); setAction({ database: db, backup: job.id }) }}>Restore this database only</Button>}</div>)}</div>)}
    <Dialog open={!!action} onOpenChange={open => { if (!busy && !open) setAction(null) }}><DialogContent className="max-h-[85svh] overflow-y-auto"><DialogHeader><DialogTitle>{action?.remove ? 'Delete logical database and SQL account' : action?.revoke ? 'Revoke database account' : action?.rotate ? 'Rotate application password' : action?.backup ? 'Restore logical database' : 'Update application connection settings'}</DialogTitle><DialogDescription>{action?.remove ? 'Permanently deletes this database and its SQL account. The linked application will lose database access. Application removal alone retains data. Existing backup history blocks deletion so its mapping remains recoverable.' : action?.rotate ? 'This invalidates the current password for this application only. Preview and apply the new settings, then redeploy the application. Other database accounts remain unchanged.' : action?.backup ? 'This imports the selected backup into this logical database in place. Pause this application and external writers first. Sibling databases and account passwords are preserved.' : action?.revoke ? 'New connections will be denied. Existing SQL sessions can remain connected. Data and other application accounts are retained.' : 'Only DB_HOST, DB_PORT, DB_DATABASE, DB_USERNAME and DB_PASSWORD are replaced. Other variables are preserved. Redeploy the application to apply these settings.'}</DialogDescription></DialogHeader>
      {action?.preview && <><Label htmlFor="database-preview">Environment after update (contains secrets)</Label><Textarea id="database-preview" value={action.preview.after} readOnly rows={9} spellCheck={false} /></>}
      <Label htmlFor="database-confirm">Type {action?.database.database_name} to confirm</Label><Input id="database-confirm" value={confirmation} onChange={event => setConfirmation(event.target.value)} autoComplete="off" />
      <DialogFooter><Button variant="outline" disabled={busy} onClick={() => setAction(null)}>Cancel</Button><LoadingButton loading={busy} loadingText="Processing…" disabled={busy || confirmation !== action?.database.database_name} onClick={() => void run(async () => { if (!action) return; await request(`${base}/${action.database.id}${action.remove ? '' : '/' + (action.revoke ? 'revoke' : action.rotate ? 'rotate' : action.backup ? `backups/${action.backup}/restore` : 'environment')}`, { ...post({ database_name: confirmation }), method: action.remove ? 'DELETE' : 'POST' }); toast.success(action.remove ? 'Logical database and account deleted' : action.rotate ? 'Password rotated. Apply connection settings and redeploy the application.' : action.backup ? 'Logical database restore queued' : action.revoke ? 'Database account revoked' : 'Saved. Redeploy the application to apply connection settings.'); setAction(null) })}>{action?.remove ? 'Permanently delete database' : action?.revoke ? 'Revoke account' : action?.rotate ? 'Rotate password' : action?.backup ? 'Restore database' : 'Save connection settings'}</LoadingButton></DialogFooter>
    </DialogContent></Dialog>
  </CardContent></Card>
}

export function ApplicationDatabase({ serviceId }: { serviceId: string }) {
  const [settings, setSettings] = useState('')
  const [error, setError] = useState('')
  useEffect(() => { setSettings(''); setError('') }, [serviceId])
  return <Card><CardHeader><CardTitle><h2>Managed database connection</h2></CardTitle><CardDescription>Only this application's restricted account is available here. Shared MariaDB root credentials remain manager-only.</CardDescription></CardHeader><CardContent className="grid gap-3"><Button variant="outline" className="justify-self-start" onClick={async () => { try { const result = await request<{ environment: string }>(`/services/${serviceId}/database-connection`); setSettings(result.environment); setError('') } catch (err) { setError(err instanceof ApiRequestError && err.status === 404 ? 'No managed database is linked to this application.' : err instanceof Error ? err.message : 'Cannot load connection settings.') } }}>{settings ? 'Refresh connection settings' : 'Show connection settings'}</Button>{error && <p role="status" className="text-sm text-muted-foreground">{error}</p>}{settings && <><Textarea aria-label="Restricted database connection settings" readOnly value={settings} rows={6} spellCheck={false} /><Button variant="ghost" className="justify-self-start" onClick={() => setSettings('')}>Hide credentials</Button></>}</CardContent></Card>
}
