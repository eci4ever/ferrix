import { useEffect, useState } from 'react'
import type { FormEvent } from 'react'
import { Link } from '@tanstack/react-router'
import { BackupPolicy } from './backup-policy'
import { DatabaseBackup, RotateCcw } from 'lucide-react'
import { request } from '@/lib/api'
import { DataTable } from '@/components/data-table'
import { Alert, AlertDescription, AlertTitle } from '@/components/ui/alert'
import { Badge } from '@/components/ui/badge'
import { Button } from '@/components/ui/button'
import {
  Card,
  CardContent,
  CardDescription,
  CardHeader,
  CardTitle,
} from '@/components/ui/card'
import {
  Dialog,
  DialogContent,
  DialogDescription,
  DialogFooter,
  DialogHeader,
  DialogTitle,
} from '@/components/ui/dialog'
import {
  Field,
  FieldDescription,
  FieldGroup,
  FieldLabel,
} from '@/components/ui/field'
import { Input } from '@/components/ui/input'
import { Skeleton } from '@/components/ui/skeleton'

type DatabaseJob = {
  id: string
  application_id: string
  service_name: string
  project_id: string
  source_backup_id: string | null
  operation: 'backup' | 'restore'
  engine: string
  status: 'queued' | 'running' | 'succeeded' | 'failed'
  message: string
  storage?: 'local' | 's3'
  scheduled_for?: string | null
  pruning?: boolean
  pruned_at?: string | null
  retention_error?: string | null
  size_bytes: number | null
  created_at: string
  started_at: string | null
  finished_at: string | null
}
export function ServiceBackups({
  serviceId,
  serviceName,
  canManage,
}: {
  serviceId: string
  serviceName: string
  canManage: boolean
}) {
  const [jobs, setJobs] = useState<DatabaseJob[] | null>(null)
  const [error, setError] = useState('')
  const [pollError, setPollError] = useState('')
  const [notice, setNotice] = useState('')
  const [busy, setBusy] = useState(false)
  const [restoring, setRestoring] = useState<DatabaseJob | null>(null)
  const base = `/services/${serviceId}/backups`
  useEffect(() => {
    const controller = new AbortController()
    let inFlight = false
    async function refresh() {
      if (document.hidden || inFlight) return
      inFlight = true
      try {
        const data = await request<DatabaseJob[]>(base, {
          signal: controller.signal,
        })
        if (!controller.signal.aborted) {
          setJobs(data)
          setPollError('')
        }
      } catch (e) {
        if (!controller.signal.aborted)
          setPollError(
            e instanceof Error ? e.message : 'Could not read backup status.',
          )
      } finally {
        inFlight = false
      }
    }
    void refresh()
    const timer = window.setInterval(() => void refresh(), 3000)
    document.addEventListener('visibilitychange', refresh)
    return () => {
      controller.abort()
      window.clearInterval(timer)
      document.removeEventListener('visibilitychange', refresh)
    }
  }, [base])
  const pending =
    jobs?.some((job) => job.status === 'queued' || job.status === 'running') ??
    false
  async function action(work: () => Promise<void>) {
    setBusy(true)
    setError('')
    setNotice('')
    try {
      await work()
      setJobs(await request<DatabaseJob[]>(base))
    } catch (e) {
      setError(
        e instanceof Error ? e.message : 'Could not queue database operation.',
      )
    } finally {
      setBusy(false)
    }
  }
  async function restore(event: FormEvent<HTMLFormElement>) {
    event.preventDefault()
    const name = String(new FormData(event.currentTarget).get('name') ?? '')
    await action(async () => {
      if (!restoring) return
      await request(`${base}/${restoring.id}/restore`, {
        method: 'POST',
        headers: { 'Content-Type': 'application/json' },
        body: JSON.stringify({ name }),
      })
      setRestoring(null)
      setNotice(
        'Restore requested. Status and the new service are listed below.',
      )
    })
  }
  return (
    <div className="flex flex-col gap-4">
      {error || pollError ? (
        <Alert variant="destructive">
          <AlertTitle>Unable to continue</AlertTitle>
          <AlertDescription>{error || pollError}</AlertDescription>
        </Alert>
      ) : null}
      {notice ? (
        <p role="status" className="text-sm">
          {notice}
        </p>
      ) : null}
      <Card>
        <CardHeader>
          <CardTitle>
            <h2>Database backups</h2>
          </CardTitle>
          <CardDescription>
            Back up the configured database. Restore creates a new Database
            service in this environment with new credentials.
          </CardDescription>
        </CardHeader>
        <CardContent className="flex flex-col gap-4">
          <p className="text-sm text-muted-foreground">
            Backups use the destination configured below. S3 backups keep a local
            copy and are only marked successful after upload completes.
            Avoid schema changes during a backup. MariaDB consistency requires
            transactional tables.
          </p>
          {canManage ? (
            <Button
              className="self-start"
              disabled={busy || pending || jobs === null}
              onClick={() =>
                void action(async () => {
                  await request(base, { method: 'POST' })
                  setNotice('Backup request accepted.')
                })
              }
            >
              <DatabaseBackup data-icon="inline-start" />
              Backup now
            </Button>
          ) : null}
          {pending ? (
            <p role="status" className="text-sm">
              A database operation is in progress. Deploy and runtime controls
              are paused for the affected services.
            </p>
          ) : null}
        </CardContent>
      </Card>
      {canManage ? <BackupPolicy serviceId={serviceId} /> : null}
      <Card>
        <CardHeader>
          <CardTitle>
            <h2>Backup and restore history</h2>
          </CardTitle>
          <CardDescription>
            Latest 100 operations. Status refreshes every 3 seconds.
          </CardDescription>
        </CardHeader>
        <CardContent>
          {jobs === null ? (
            <Skeleton className="h-32 w-full" />
          ) : (
            <DataTable
              label="Backup history"
              data={jobs}
              initialSorting={[{ id: 'created', desc: true }]}
              columns={[
                {
                  id: 'operation',
                  label: 'Operation',
                  value: (job) => job.operation,
                  cell: (job) =>
                    job.operation === 'backup' ? (job.scheduled_for ? 'Scheduled backup' : 'Backup') : 'Restore',
                },
                {
                  id: 'status',
                  label: 'Status',
                  value: (job) => job.status,
                  cell: (job) => (
                    <Badge
                      variant={
                        job.status === 'failed'
                          ? 'destructive'
                          : job.status === 'succeeded'
                            ? 'secondary'
                            : 'outline'
                      }
                    >
                      {job.status}
                    </Badge>
                  ),
                },
                {
                  id: 'storage', label: 'Storage', value: (job) => job.pruned_at ? 'Removed' : job.pruning ? 'Removal pending' : job.storage ?? 'local', cell: (job) => job.pruned_at ? 'Removed by retention' : job.pruning ? 'Removal pending' : job.storage === 's3' ? 'Local + S3' : 'Local',
                },
                {
                  id: 'created',
                  label: 'Created',
                  value: (job) => job.created_at,
                  cell: (job) => (
                    <time dateTime={job.created_at}>
                      {new Date(job.created_at).toLocaleString()}
                    </time>
                  ),
                },
                {
                  id: 'size',
                  label: 'Size',
                  value: (job) => String(job.size_bytes ?? 0).padStart(20, '0'),
                  cell: (job) =>
                    job.size_bytes === null
                      ? '—'
                      : job.size_bytes < 1024 * 1024
                        ? `${(job.size_bytes / 1024).toFixed(1)} KiB`
                        : `${(job.size_bytes / 1024 / 1024).toFixed(2)} MiB`,
                },
                {
                  id: 'progress',
                  label: 'Progress',
                  value: (job) => job.message,
                  cell: (job) => (
                    <span className="whitespace-normal">{job.message}{job.retention_error ? <span className="block text-destructive">{job.retention_error}</span> : null}</span>
                  ),
                },
                {
                  id: 'service',
                  label: 'Service',
                  value: (job) => job.service_name,
                  cell: (job) =>
                    job.operation === 'restore' ? (
                      <Button asChild variant="link" size="sm">
                        <Link
                          to="/projects/$projectId/services/$serviceId"
                          params={{
                            projectId: job.project_id,
                            serviceId: job.application_id,
                          }}
                        >
                          {job.service_name}
                        </Link>
                      </Button>
                    ) : (
                      job.service_name
                    ),
                },
                ...(canManage
                  ? [
                      {
                        id: 'restore',
                        label: 'Restore',
                        cell: (job: DatabaseJob) =>
                          job.operation === 'backup' &&
                          job.status === 'succeeded' &&
                          !job.pruning && !job.pruned_at &&
                          job.application_id === serviceId ? (
                            <Button
                              variant="outline"
                              size="sm"
                              disabled={busy || pending}
                              onClick={() => setRestoring(job)}
                            >
                              <RotateCcw data-icon="inline-start" />
                              Restore
                            </Button>
                          ) : null,
                      },
                    ]
                  : []),
              ]}
            />
          )}
        </CardContent>
      </Card>
      <Dialog
        open={restoring !== null}
        onOpenChange={(open) => {
          if (!open && !busy) setRestoring(null)
        }}
      >
        <DialogContent>
          <DialogHeader>
            <DialogTitle>Restore to a new database</DialogTitle>
            <DialogDescription>
              A new Database service will be created in the same environment,
              using this backup’s database version and a fresh password. Wait
              for restore to succeed before using it.
            </DialogDescription>
          </DialogHeader>
          <form onSubmit={restore} className="flex flex-col gap-4">
            {restoring ? (
              <p className="text-sm text-muted-foreground">
                Backup from {new Date(restoring.created_at).toLocaleString()}
              </p>
            ) : null}
            <FieldGroup>
              <Field>
                <FieldLabel htmlFor="restore-service-name">
                  New service name
                </FieldLabel>
                <Input
                  id="restore-service-name"
                  name="name"
                  defaultValue={`${serviceName.slice(0, 65)} restored`}
                  required
                  maxLength={80}
                  disabled={busy}
                />
                <FieldDescription>
                  The database name and username match the backup. Connection
                  details are available on the new service.
                </FieldDescription>
              </Field>
            </FieldGroup>
            {error ? (
              <p role="alert" className="text-sm text-destructive">
                {error}
              </p>
            ) : null}
            <DialogFooter>
              <Button
                type="button"
                variant="outline"
                disabled={busy}
                onClick={() => setRestoring(null)}
              >
                Cancel
              </Button>
              <Button disabled={busy || pending}>
                {busy ? 'Queuing…' : 'Restore backup'}
              </Button>
            </DialogFooter>
          </form>
        </DialogContent>
      </Dialog>
    </div>
  )
}
