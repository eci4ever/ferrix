import { useState } from 'react'
import { Link } from '@tanstack/react-router'
import { request } from '@/lib/api'
import { Card, CardHeader, CardTitle, CardContent } from '@/components/ui/card'
import { Field, FieldLabel, FieldDescription } from '@/components/ui/field'
import { Input } from '@/components/ui/input'
import { Button } from '@/components/ui/button'
import { RadioGroup, RadioGroupItem } from '@/components/ui/radio-group'

export type Profile = { name: string; email: string; avatar: string }

export function ProfilePage({ profile, refresh }: { profile: Profile; refresh: () => Promise<unknown> }) {
  const [name, setName] = useState(profile.name)
  const [avatar, setAvatar] = useState(profile.avatar)
  const [busy, setBusy] = useState(false)
  const [error, setError] = useState('')
  const [notice, setNotice] = useState('')
  return <section className="grid max-w-2xl gap-6">
    <h1 className="text-2xl font-semibold">Profile</h1>
    <Card><CardHeader><CardTitle>Personal details</CardTitle></CardHeader><CardContent>
      <form className="grid gap-4" onSubmit={async event => {
        event.preventDefault()
        setBusy(true); setError(''); setNotice('')
        try {
          await request('/auth/profile', { method: 'POST', headers: { 'Content-Type': 'application/json' }, body: JSON.stringify({ name, avatar }) })
          await refresh()
          setNotice('Profile saved.')
        } catch (reason) { setError(reason instanceof Error ? reason.message : 'Could not save profile. Try again.') }
        finally { setBusy(false) }
      }}>
        <Field><FieldLabel htmlFor="profile-name">Name</FieldLabel><Input id="profile-name" value={name} onChange={event => setName(event.target.value)} required maxLength={80} disabled={busy} /></Field>
        <Field><FieldLabel htmlFor="profile-email">Email</FieldLabel><Input id="profile-email" value={profile.email} readOnly /><FieldDescription>Your sign-in email.</FieldDescription></Field>
        <fieldset disabled={busy} className="grid gap-2"><legend className="mb-2 text-sm font-medium">Avatar</legend>
          <RadioGroup aria-label="Avatar" value={avatar} onValueChange={setAvatar} className="flex flex-wrap gap-3">
            {['initials','🌿','🚀','🐱','🌊','⭐'].map((value, index) => <label key={value} className="flex items-center gap-2 rounded-md border p-3"><RadioGroupItem value={value} aria-label={['Use initials','Leaf','Rocket','Cat','Wave','Star'][index]} /><span>{value === 'initials' ? 'Initials' : value}</span></label>)}
          </RadioGroup>
        </fieldset>
        {error && <p role="alert" className="text-sm text-destructive">{error}</p>}
        {notice && <p role="status" className="text-sm">{notice}</p>}
        <Button className="w-fit" disabled={busy}>{busy ? 'Saving…' : 'Save profile'}</Button>
      </form>
    </CardContent></Card>
    <Card><CardHeader><CardTitle>Account security</CardTitle></CardHeader><CardContent><p className="mb-4 text-sm text-muted-foreground">Manage your password, two-factor authentication and signed-in sessions.</p><Button asChild variant="outline"><Link to="/security">Manage security and sessions</Link></Button></CardContent></Card>
  </section>
}
