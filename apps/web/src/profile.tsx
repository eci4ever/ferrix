import { useState } from 'react'
import { Link } from '@tanstack/react-router'
import { request } from '@/lib/api'
import { Alert, AlertTitle, AlertDescription } from '@/components/ui/alert'
import { Field, FieldLabel, FieldDescription } from '@/components/ui/field'
import { Input } from '@/components/ui/input'
import { Button } from '@/components/ui/button'
import { RadioGroup, RadioGroupItem } from '@/components/ui/radio-group'

const panel = 'flex flex-col gap-4 rounded-xl border bg-card p-6 text-card-foreground'

export type Profile = { name: string; email: string; avatar: string; avatar_url?: string | null }

export function ProfilePage({ profile, refresh }: { profile: Profile; refresh: () => Promise<unknown> }) {
  const [name, setName] = useState(profile.name)
  const [avatar, setAvatar] = useState(profile.avatar)
  const [avatarUrl, setAvatarUrl] = useState(profile.avatar_url ?? '')
  const [busy, setBusy] = useState(false)
  const [error, setError] = useState('')
  const [notice, setNotice] = useState('')
  return <div className="flex flex-col gap-6">
    <h1 className="text-2xl font-semibold">Profile</h1>
    {error && <Alert variant="destructive" role="alert"><AlertTitle>Profile could not be saved</AlertTitle><AlertDescription>{error}</AlertDescription></Alert>}
    {notice && <Alert role="status"><AlertTitle>Account notice</AlertTitle><AlertDescription>{notice}</AlertDescription></Alert>}
    <section className={panel}><h2 className="font-semibold">Personal details</h2>
      <form className="flex flex-col gap-4" onSubmit={async event => {
        event.preventDefault()
        setBusy(true); setError(''); setNotice('')
        try {
          await request('/auth/profile', { method: 'POST', headers: { 'Content-Type': 'application/json' }, body: JSON.stringify({ name, avatar, avatar_url: avatarUrl.trim() || null }) })
          await refresh()
          setNotice('Profile saved.')
        } catch (reason) { setError(reason instanceof Error ? reason.message : 'Could not save profile. Try again.') }
        finally { setBusy(false) }
      }}>
        <Field><FieldLabel htmlFor="profile-name">Name</FieldLabel><Input id="profile-name" value={name} onChange={event => setName(event.target.value)} required maxLength={80} disabled={busy} /></Field>
        <Field><FieldLabel htmlFor="profile-email">Email</FieldLabel><Input id="profile-email" value={profile.email} readOnly /><FieldDescription>Your sign-in email.</FieldDescription></Field>
        <Field><FieldLabel htmlFor="profile-avatar-url">Profile image URL</FieldLabel><Input id="profile-avatar-url" type="url" placeholder="https://example.com/photo.jpg" value={avatarUrl} onChange={event => setAvatarUrl(event.target.value)} maxLength={2048} disabled={busy} /><FieldDescription>Use an HTTPS image link. Clear it to use your selected avatar. The image provider receives requests from your browser.</FieldDescription></Field>
        <fieldset disabled={busy} className="grid gap-2"><legend className="mb-2 text-sm font-medium">Avatar</legend>
          <RadioGroup aria-label="Avatar" value={avatar} onValueChange={setAvatar} className="flex flex-wrap gap-3">
            {['initials','🌿','🚀','🐱','🌊','⭐'].map((value, index) => <label key={value} className="flex items-center gap-2 rounded-md border p-3"><RadioGroupItem value={value} aria-label={['Use initials','Leaf','Rocket','Cat','Wave','Star'][index]} /><span>{value === 'initials' ? 'Initials' : value}</span></label>)}
          </RadioGroup>
        </fieldset>
        <Button disabled={busy}>{busy ? 'Saving…' : 'Save profile'}</Button>
      </form>
    </section>
    <section className={panel}><h2 className="font-semibold">Account security</h2><p className="text-sm text-muted-foreground">Manage your password, two-factor authentication and signed-in sessions.</p><Button asChild variant="outline"><Link to="/security">Manage security and sessions</Link></Button></section>
  </div>
}
