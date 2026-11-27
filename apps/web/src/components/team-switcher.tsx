import { useRef, useState } from 'react'
import { Link, useRouter } from '@tanstack/react-router'
import { Check, ChevronsUpDown, Layers3, Plus, Settings2 } from 'lucide-react'
import { toast } from 'sonner'
import { request } from '@/lib/api'
import { FormDialog } from '@/components/form-dialog'
import { Button } from '@/components/ui/button'
import { Input } from '@/components/ui/input'
import { Field, FieldLabel } from '@/components/ui/field'
import { Alert, AlertTitle, AlertDescription } from '@/components/ui/alert'
import { DialogFooter } from '@/components/ui/dialog'
import type { Identity, Organization } from '@/auth'
import { DropdownMenu, DropdownMenuContent, DropdownMenuGroup, DropdownMenuItem, DropdownMenuLabel, DropdownMenuSeparator, DropdownMenuTrigger } from '@/components/ui/dropdown-menu'
import { SidebarMenu, SidebarMenuButton, SidebarMenuItem, useSidebar } from '@/components/ui/sidebar'

export function TeamSwitcher({ identity, organizations, busy, onSwitch }: { identity: Identity; organizations: Organization[]; busy: boolean; onSwitch: (id: string) => Promise<void> }) {
  const { isMobile, setOpenMobile } = useSidebar()
  const router = useRouter()
  const trigger = useRef<HTMLButtonElement>(null)
  const choosing = useRef(false)
  const [creating, setCreating] = useState(false)
  const [saving, setSaving] = useState(false)
  const [error, setError] = useState('')
  const logo = organizations.find(org => org.id === identity.organization_id)?.logo_url
  return <><SidebarMenu><SidebarMenuItem><DropdownMenu><DropdownMenuTrigger asChild><SidebarMenuButton ref={trigger} size="lg" disabled={busy} tooltip={identity.organization_name} aria-label="Switch organization"><div className="flex aspect-square size-8 items-center justify-center rounded-lg bg-sidebar-accent text-sidebar-accent-foreground">{logo ? <img src={logo} alt="" className="size-6 object-contain" referrerPolicy="no-referrer" /> : <Layers3 />}</div><div className="grid flex-1 text-left text-sm leading-tight"><span className="truncate font-semibold">{identity.organization_name}</span><span className="truncate text-xs">Ferrix</span></div><ChevronsUpDown className="ml-auto" /></SidebarMenuButton></DropdownMenuTrigger><DropdownMenuContent onCloseAutoFocus={event => { if (choosing.current) { event.preventDefault(); choosing.current = false } }} align="start" side={isMobile ? 'bottom' : 'right'} sideOffset={4}><DropdownMenuLabel>Organizations</DropdownMenuLabel><DropdownMenuGroup>{organizations.map(org => <DropdownMenuItem key={org.id} disabled={busy || org.id === identity.organization_id} onSelect={() => { void onSwitch(org.id) }}><Layers3 /><span>{org.name}</span>{org.id === identity.organization_id ? <Check className="ml-auto" /> : null}</DropdownMenuItem>)}</DropdownMenuGroup><DropdownMenuSeparator /><DropdownMenuGroup><DropdownMenuItem asChild><Link to={identity.role === 'owner' || identity.role === 'admin' ? '/organization/settings' : '/organization'} onClick={() => setOpenMobile(false)}><Settings2 />Manage organization</Link></DropdownMenuItem>{identity.system_role === 'admin' ? <DropdownMenuItem disabled={busy} onSelect={() => { choosing.current = true; setError(''); setCreating(true) }}><Plus />Create organization</DropdownMenuItem> : null}</DropdownMenuGroup></DropdownMenuContent></DropdownMenu></SidebarMenuItem></SidebarMenu>
    <FormDialog open={creating} onOpenChange={open => { setCreating(open); setError('') }} title="Create organization" description="Create an organization to group projects and members. Your current organization stays selected." busy={saving} onCloseAutoFocus={event => { event.preventDefault(); trigger.current?.focus() }}>
      {error ? <Alert variant="destructive" role="alert"><AlertTitle>Organization could not be created</AlertTitle><AlertDescription>{error}</AlertDescription></Alert> : null}
      <form className="flex flex-col gap-4" onSubmit={async event => {
        event.preventDefault()
        const name = new FormData(event.currentTarget).get('name')
        setSaving(true); setError('')
        try {
          await request('/auth/organizations', { method: 'POST', notifyOnError: true, headers: { 'Content-Type': 'application/json' }, body: JSON.stringify({ name }) })
          setCreating(false)
          toast.success('Organization created.')
          await router.invalidate()
        } catch (reason) { setError(reason instanceof Error ? reason.message : 'Could not create organization.') }
        finally { setSaving(false) }
      }}>
        <fieldset disabled={saving}><Field><FieldLabel htmlFor="new-organization-name">New organization name</FieldLabel><Input id="new-organization-name" name="name" required maxLength={80} autoFocus /></Field></fieldset>
        <DialogFooter><Button type="button" variant="outline" disabled={saving} onClick={() => setCreating(false)}>Cancel</Button><Button type="submit" disabled={saving}>{saving ? 'Creating…' : 'Create organization'}</Button></DialogFooter>
      </form>
    </FormDialog>
  </>
}
