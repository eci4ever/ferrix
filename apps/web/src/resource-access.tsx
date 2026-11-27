import { LoadingButton } from '@/components/loading-button'
import { useRef, useState } from 'react'
import { toast } from 'sonner'
import { request } from '@/lib/api'
import { FormDialog } from '@/components/form-dialog'
import { Alert, AlertTitle, AlertDescription } from '@/components/ui/alert'
import { Button } from '@/components/ui/button'
import { Card, CardHeader, CardTitle, CardDescription, CardContent } from '@/components/ui/card'
import { Field, FieldLabel } from '@/components/ui/field'
import { NativeSelect, NativeSelectOption } from '@/components/ui/native-select'
import { Combobox, ComboboxInput, ComboboxContent, ComboboxList, ComboboxItem, ComboboxEmpty } from '@/components/ui/combobox'
import { Checkbox } from '@/components/ui/checkbox'
type Member = { id: string; name: string; email: string; role: string }
type Grant = { user_id: string; permission: string }
const post = (body: unknown) => ({ method: 'POST', headers: { 'Content-Type': 'application/json' }, body: JSON.stringify(body) })
function Failure({ error }: { error: string }) { return error ? <Alert variant="destructive" role="alert"><AlertTitle>Access could not be updated</AlertTitle><AlertDescription>{error}</AlertDescription></Alert> : null }

export function ResourceAccess({ resource, canManage }: { resource: string; canManage: boolean }) {
  const memberPopup = useRef<HTMLDivElement>(null)
  const [open, setOpen] = useState(false), [busy, setBusy] = useState(false), [error, setError] = useState('')
  const [members, setMembers] = useState<Member[]>([]), [grants, setGrants] = useState<Grant[]>([])
  const [member, setMember] = useState(''), [permission, setPermission] = useState('viewer'), [loaded, setLoaded] = useState(false)
  async function load() {
    setBusy(true); setError(''); setLoaded(false)
    try { const [people, access] = await Promise.all([request<Member[]>('/auth/members'), request<{ assignments: Grant[] }>(`${resource}/assignments`)]); setMembers(people); setGrants(access.assignments); setLoaded(true) }
    catch (reason) { setError(reason instanceof Error ? reason.message : 'Could not load access.') }
    finally { setBusy(false) }
  }
  if (!canManage) return null
  return <Card><CardHeader><CardTitle><h2>Resource access</h2></CardTitle><CardDescription>{resource.startsWith('/projects/') ? 'Project assignments cover every service in this project. Assign students to individual services for a shared class project.' : 'Viewer can read overview and logs. Developer can manage this application, its source and environment; resource limits, database administration, backups and destructive actions remain manager-only.'}</CardDescription></CardHeader><CardContent>
    <FormDialog open={open} onOpenChange={value => { setOpen(value); if (value) { setMember(''); setPermission('viewer'); void load() } }} title="Manage resource access" description="Assign only members of this organization. Revoking a direct assignment does not remove inherited project access or legacy organization visibility." busy={busy} trigger={<Button variant="outline">Manage access</Button>}>
      <Failure error={error} />
      {!loaded && !busy ? <Button variant="outline" onClick={() => void load()}>Retry access</Button> : null}
      {loaded ? <form className="flex flex-col gap-4" onSubmit={async event => { event.preventDefault(); if (busy || !member) return; setBusy(true); setError(''); try { await request(`${resource}/assignments`, post({ user_id: member, permission: permission === 'none' ? null : permission })); const result = await request<{ assignments: Grant[] }>(`${resource}/assignments`); setGrants(result.assignments); toast.success('Resource access saved.') } catch (reason) { setError(reason instanceof Error ? reason.message : 'Could not save access.') } finally { setBusy(false) } }}>
        <Field><FieldLabel htmlFor="assignment-member">Organization member</FieldLabel><div ref={memberPopup} className="min-w-0"><Combobox items={members} value={members.find(person => person.id === member) ?? null} itemToStringLabel={person => `${person.name} (${person.email}) · ${person.role}`} disabled={busy} onValueChange={person => { const value = person?.id ?? ''; setMember(value); setPermission(grants.find(grant => grant.user_id === value)?.permission ?? 'none') }}><ComboboxInput id="assignment-member" placeholder="Search members by name or email…" className="w-full min-w-0" showClear /><ComboboxContent container={memberPopup} className="min-w-0"><ComboboxEmpty>No matching members.</ComboboxEmpty><ComboboxList>{(person: Member) => <ComboboxItem key={person.id} value={person} className="items-start py-2"><span className="min-w-0 break-words"><span className="block font-medium">{person.name}</span><span className="block text-xs text-muted-foreground">{person.email} · {person.role}</span></span></ComboboxItem>}</ComboboxList></ComboboxContent></Combobox></div></Field>
        <Field><FieldLabel htmlFor="assignment-permission">Resource permission</FieldLabel><NativeSelect id="assignment-permission" value={permission} onChange={event => setPermission(event.target.value)} disabled={busy}><NativeSelectOption value="none">No direct assignment</NativeSelectOption><NativeSelectOption value="viewer">Viewer</NativeSelectOption><NativeSelectOption value="developer">Developer</NativeSelectOption></NativeSelect></Field>
        <p className="text-sm text-muted-foreground">Organization owners and admins retain management access. Project and service grants combine; the stronger grant applies. Database credentials and terminals remain manager-only.</p>
        <div className="flex justify-end gap-2"><Button type="button" variant="outline" disabled={busy} onClick={() => setOpen(false)}>Close</Button><LoadingButton loading={busy} loadingText="Saving…" disabled={busy || !member}>{busy ? 'Saving…' : 'Save access'}</LoadingButton></div>
      </form> : null}
    </FormDialog>
  </CardContent></Card>
}

export function ClassroomAccess({ canManage }: { canManage: boolean }) {
  const [restricted, setRestricted] = useState<boolean | null>(null), [busy, setBusy] = useState(false), [error, setError] = useState('')
  if (!canManage) return null
  return <Card><CardHeader><CardTitle><h2>Member resource visibility</h2></CardTitle><CardDescription>Existing organizations keep read-only member visibility until restriction is enabled. Explicit developer assignments grant approved application actions in either mode.</CardDescription></CardHeader><CardContent className="flex flex-col gap-4"><Failure error={error} />
    {restricted === null ? <Button variant="outline" className="self-start" disabled={busy} onClick={async () => { setBusy(true); setError(''); try { setRestricted((await request<{ restricted: boolean }>('/auth/organizations/access')).restricted) } catch (reason) { setError(reason instanceof Error ? reason.message : 'Could not load visibility.') } finally { setBusy(false) } }}>Configure member visibility</Button> : <form className="flex flex-col gap-4" onSubmit={async event => { event.preventDefault(); setBusy(true); setError(''); try { await request('/auth/organizations/access', post({ restricted })); toast.success('Member visibility saved.') } catch (reason) { setError(reason instanceof Error ? reason.message : 'Could not save visibility.') } finally { setBusy(false) } }}>
      <label className="flex items-start gap-3 text-sm"><Checkbox checked={restricted} disabled={busy} onCheckedChange={value => setRestricted(value === true)} />Restrict members to assigned projects and services</label>
      <p className="text-sm text-muted-foreground">Enable for classrooms. Unassigned members cannot enumerate classmates’ resources. Owners/admins retain access; disabling this restores organization-wide read-only visibility.</p><LoadingButton loading={busy} loadingText="Saving…" className="self-start" disabled={busy}>Save member visibility</LoadingButton>
    </form>}
  </CardContent></Card>
}
