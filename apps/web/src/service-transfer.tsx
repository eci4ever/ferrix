import { useEffect, useRef, useState } from 'react'
import type { FormEvent } from 'react'
import { request } from '@/lib/api'
import { FormDialog } from '@/components/form-dialog'
import { Alert, AlertDescription, AlertTitle } from '@/components/ui/alert'
import { Button } from '@/components/ui/button'
import { DialogFooter } from '@/components/ui/dialog'
import { Field, FieldDescription, FieldGroup, FieldLabel } from '@/components/ui/field'
import { Input } from '@/components/ui/input'
import { NativeSelect, NativeSelectOption } from '@/components/ui/native-select'

type Environment = { id: string; name: string }
export function ServiceTransfer({ serviceId, projectId, environmentId, name, disabled }: { serviceId: string; projectId: string; environmentId: string; name: string; disabled: boolean }) {
  const [mode, setMode] = useState<'move' | 'clone' | null>(null)
  const [environments, setEnvironments] = useState<Environment[]>([])
  const [target, setTarget] = useState('')
  const [label, setLabel] = useState(name)
  const [loading, setLoading] = useState(false)
  const [busy, setBusy] = useState(false)
  const [error, setError] = useState('')
  const [retry, setRetry] = useState(0)
  const origin = useRef<HTMLButtonElement | null>(null)
  useEffect(() => {
    if (!mode) return
    const abort = new AbortController()
    setLoading(true); setError(''); setTarget(''); setLabel(name)
    request<{ environments: Environment[] }>(`/projects/${projectId}/environments`, { signal: abort.signal }).then(value => {
      if (!abort.signal.aborted) setEnvironments(value.environments.filter(row => row.id !== environmentId))
    }).catch(e => { if (!abort.signal.aborted) setError(e instanceof Error ? e.message : 'Could not load environments.') }).finally(() => { if (!abort.signal.aborted) setLoading(false) })
    return () => abort.abort()
  }, [mode, projectId, environmentId, name, retry])
  async function submit(event: FormEvent) {
    event.preventDefault()
    if (!mode || !target || busy) return
    setBusy(true); setError('')
    try {
      const service = await request<{id:string}>(`/services/${serviceId}/${mode}`, { method:'POST', headers:{'Content-Type':'application/json'}, body:JSON.stringify({environment_id:target, ...(mode === 'clone' ? {name:label} : {})}), signal:AbortSignal.timeout(30_000) })
      window.location.assign(`/projects/${projectId}/services/${service.id}`)
    } catch (e) { setError(e instanceof Error ? e.message : 'Could not complete the operation.'); setBusy(false) }
  }
  return <>
    <Button variant="outline" disabled={disabled} onClick={event => { origin.current = event.currentTarget; setMode('move') }}>Move service</Button>
    <Button variant="outline" disabled={disabled} onClick={event => { origin.current = event.currentTarget; setMode('clone') }}>Clone to environment</Button>
    <FormDialog open={mode !== null} onOpenChange={open => { if (!open) setMode(null) }} busy={busy} title={mode === 'move' ? 'Move service' : 'Clone to environment'} description={mode === 'move' ? 'Move this service to another environment in the same project.' : 'Create an undeployed copy in another environment in the same project.'} onCloseAutoFocus={event => { event.preventDefault(); origin.current?.focus() }}>
      <form onSubmit={submit} className="flex flex-col gap-4">
        <FieldGroup><Field><FieldLabel htmlFor="transfer-environment">Destination environment</FieldLabel><NativeSelect id="transfer-environment" autoFocus required value={target} onChange={event => setTarget(event.target.value)} disabled={loading || busy} className="w-full"><NativeSelectOption value="">{loading ? 'Loading environments…' : 'Select an environment'}</NativeSelectOption>{environments.map(row => <NativeSelectOption key={row.id} value={row.id}>{row.name}</NativeSelectOption>)}</NativeSelect><FieldDescription>{!loading && !error && !environments.length ? 'Create another environment in this project first.' : 'Only other environments in this project are available.'}</FieldDescription></Field>{mode === 'clone' ? <Field><FieldLabel htmlFor="clone-service-name">Service name</FieldLabel><Input id="clone-service-name" value={label} onChange={event => setLabel(event.target.value)} required maxLength={80} disabled={busy} /></Field> : null}</FieldGroup>
        <Alert><AlertTitle>{mode === 'move' ? 'Stop before moving' : 'Review the copy before deploying'}</AlertTitle><AlertDescription>{mode === 'move' ? 'Stop any deployed container first. The stopped container is removed, but its data volume, notes, domains and deployment history are retained. Deploy again to join the destination network. Connected services and environment variables are not changed automatically.' : 'Saved settings, source credentials, environment variables and notes are copied. Database passwords are regenerated. Data, domains, deployment history, backup schedules and webhook auto-deploy are not copied. Storage uses a new volume. Review copied connection strings before deploying.'}</AlertDescription></Alert>
        {error ? <Alert variant="destructive"><AlertTitle>Unable to continue</AlertTitle><AlertDescription><p>{error}</p>{!environments.length ? <Button type="button" variant="outline" disabled={loading || busy} onClick={() => setRetry(value => value + 1)}>Retry loading environments</Button> : null}</AlertDescription></Alert> : null}
        <DialogFooter><Button type="button" variant="outline" disabled={busy} onClick={() => setMode(null)}>Cancel</Button><Button disabled={busy || loading || !target || (mode === 'clone' && !label.trim())}>{busy ? 'Working…' : mode === 'move' ? 'Move service' : 'Clone service'}</Button></DialogFooter>
      </form>
    </FormDialog>
  </>
}
