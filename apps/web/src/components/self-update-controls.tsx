import { useEffect, useState } from 'react'
import { ApiRequestError, request } from '@/lib/api'
import { Button } from '@/components/ui/button'
import { Alert, AlertDescription, AlertTitle } from '@/components/ui/alert'
import { Checkbox } from '@/components/ui/checkbox'
import { Field, FieldGroup, FieldLabel } from '@/components/ui/field'
import { Input } from '@/components/ui/input'

export type UpdateStatus = { supported: boolean; active: boolean; stage: string; target_version: string | null; backup_path: string | null }
const stages: Record<string, string> = { preparing: 'Preparing updater', pulling: 'Downloading the release image', backing_up: 'Backing up PostgreSQL and configuration', updating: 'Replacing the Ferrix container', verifying: 'Checking Ferrix health', succeeded: 'Update completed', rolling_back: 'Recovering the previous image', rolled_back: 'Update failed; previous image recovered', failed: 'Update failed. Review the upgrade guide and host update log.' }

export function useSelfUpdate() {
  const [status, setStatus] = useState<UpdateStatus | null>(null)
  const [busy, setBusy] = useState(false)
  const [seen, setSeen] = useState(false)
  const [error, setError] = useState('')
  const [connectionError, setConnectionError] = useState('')
  const active = busy || Boolean(status?.active)
  useEffect(() => {
    const controller = new AbortController()
    let loading = false
    const refresh = async () => {
      if (loading) return
      if (document.visibilityState === 'hidden') return
      loading = true
      try {
        const next = await request<UpdateStatus>('/releases/update', { signal: AbortSignal.any([controller.signal, AbortSignal.timeout(10_000)]) })
        if (controller.signal.aborted) return
        setStatus(next)
        if (next.active) setSeen(true)
        setConnectionError('')
      } catch {
        if (!controller.signal.aborted && active) setConnectionError('Waiting for Ferrix to reconnect. The independent updater continues while Ferrix restarts. If it does not return, follow the upgrade guide on the VPS.')
      } finally { loading = false }
    }
    void refresh()
    const timer = window.setInterval(() => void refresh(), active ? 3_000 : 15 * 60_000)
    document.addEventListener('visibilitychange', refresh)
    return () => { controller.abort(); window.clearInterval(timer); document.removeEventListener('visibilitychange', refresh) }
  }, [active])

  async function start(version: string, values: Record<string, unknown>) {
    setBusy(true); setError(''); setConnectionError('')
    try {
      const next = await request<UpdateStatus>('/releases/update', { method: 'POST', headers: { 'Content-Type': 'application/json' }, signal: AbortSignal.timeout(180_000), body: JSON.stringify({ ...values, version }) })
      setStatus(next); setSeen(true)
    } catch (error) {
      if (!(error instanceof ApiRequestError)) {
        setStatus({ supported: true, active: true, stage: 'preparing', target_version: version, backup_path: null }); setSeen(true)
        setConnectionError('Checking whether the updater started. Ferrix will reconnect automatically.')
      } else { setError(error.message) }
    }
    finally { setBusy(false) }
  }
  return { status, active, busy, seen, error: error || connectionError, start }
}

export function SelfUpdateControls({ update, version, available }: { update: ReturnType<typeof useSelfUpdate>; version: string | null; available: boolean }) {
  const [acknowledged, setAcknowledged] = useState(false)
  const [confirmation, setConfirmation] = useState('')
  return <section className="flex flex-col gap-4" aria-label="Update Ferrix">
    {update.status?.stage && update.status.stage !== 'idle' ? <Alert><AlertTitle role="status">{stages[update.status.stage] ?? 'Checking update status'}</AlertTitle><AlertDescription>{update.status.target_version ? <p>Target: v{update.status.target_version}</p> : null}{update.status.backup_path ? <p className="break-all">Backup and log: {update.status.backup_path}</p> : null}</AlertDescription></Alert> : null}
    {update.status?.stage === 'succeeded' ? <Button type="button" onClick={() => window.location.reload()}>Reload Ferrix</Button> : null}
    {update.error ? <Alert variant="destructive"><AlertDescription role="alert">{update.error}</AlertDescription></Alert> : null}
    {!update.status?.supported ? <p className="text-sm text-muted-foreground">UI updates are available for managed production installations. Use the upgrade guide for development or custom installations.</p> : available && !update.active ? <form className="flex flex-col gap-4" onSubmit={async event => {
      event.preventDefault()
      const values = Object.fromEntries(new FormData(event.currentTarget))
      if (version) await update.start(version, { ...values, backup_acknowledged: acknowledged })
    }}>
      <p className="text-sm text-muted-foreground">Ferrix will back up its PostgreSQL database and configuration, then restart itself. Service containers keep running, but website routing may be briefly unavailable. Keep separate backups of service data. Automatic recovery to the old image is attempted only when the database schema is unchanged.</p>
      <FieldGroup>
        <Field><FieldLabel htmlFor="update-password">Current password</FieldLabel><Input id="update-password" name="password" type="password" autoComplete="current-password" required maxLength={1024} /></Field>
        <Field><FieldLabel htmlFor="update-code">Authenticator or recovery code (if enabled)</FieldLabel><Input id="update-code" name="code" autoComplete="one-time-code" maxLength={128} /></Field>
        <Field orientation="horizontal"><Checkbox id="update-backup" checked={acknowledged} onCheckedChange={value => setAcknowledged(value === true)} /><FieldLabel htmlFor="update-backup">I have reviewed the release notes and understand the backup and recovery policy.</FieldLabel></Field>
        <Field><FieldLabel htmlFor="update-confirmation">Type UPDATE FERRIX to confirm</FieldLabel><Input id="update-confirmation" name="confirmation" value={confirmation} onChange={event => setConfirmation(event.target.value)} autoComplete="off" required /></Field>
      </FieldGroup>
      <Button type="submit" disabled={update.busy || !acknowledged || confirmation !== 'UPDATE FERRIX'}>Update to v{version}</Button>
    </form> : update.active ? <p className="text-sm text-muted-foreground" role="status">{update.busy ? 'Preparing the update. This may take a few minutes.' : 'Update in progress. Ferrix may be unavailable briefly; this page will reconnect automatically.'}</p> : null}
  </section>
}
