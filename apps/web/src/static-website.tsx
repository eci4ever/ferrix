import { useEffect, useState } from 'react'
import type { FormEvent } from 'react'
import { Link } from '@tanstack/react-router'
import { Alert, AlertDescription, AlertTitle } from '@/components/ui/alert'
import { Button } from '@/components/ui/button'
import { Card, CardContent, CardDescription, CardHeader, CardTitle } from '@/components/ui/card'
import { Field, FieldDescription, FieldLabel } from '@/components/ui/field'
import { Input } from '@/components/ui/input'
import { Badge } from '@/components/ui/badge'
import { request } from '@/lib/api'

export type StaticSite = {
  id: string
  project_id: string
  name: string
  automatic_url?: string | null
  domain: string | null
  active_release_id: string | null
  created_at: string
}

type SiteDetails = {
  site: StaticSite
  releases: { id: string; bytes: number; created_at: string }[]
}

const MAX_BYTES = 8 * 1024 * 1024
const HEX_BYTES = Array.from({ length: 256 }, (_, byte) => byte.toString(16).padStart(2, '0'))

async function encodeBuild(files: File[]) {
  if (!files.length || files.length > 512 || files.reduce((sum, file) => sum + file.size, 0) > MAX_BYTES) {
    throw new Error('Select a build folder with up to 512 files and 8 MiB total.')
  }
  const build = []
  for (const file of files) {
    const path = file.webkitRelativePath.split('/').slice(1).join('/')
    const bytes = new Uint8Array(await file.arrayBuffer())
    let hex = ''
    for (let start = 0; start < bytes.length; start += 8192) {
      hex += Array.from(bytes.subarray(start, start + 8192), byte => HEX_BYTES[byte]).join('')
    }
    build.push({ path, hex })
  }
  return build
}

export function StaticWebsite({ projectId, siteId, canManage, canSetDomain }: {
  projectId: string
  siteId: string
  canManage: boolean
  canSetDomain: boolean
}) {
  const [details, setDetails] = useState<SiteDetails | null>(null)
  const [domain, setDomain] = useState('')
  const [busy, setBusy] = useState(false)
  const [error, setError] = useState('')
  const [notice, setNotice] = useState('')
  const path = `/static-websites/${siteId}`

  useEffect(() => {
    const controller = new AbortController()
    request<SiteDetails>(path, { signal: controller.signal })
      .then(result => {
        if (controller.signal.aborted) return
        setDetails(result)
        setDomain(result.site.domain ?? '')
      })
      .catch(reason => { if (!controller.signal.aborted) setError(reason.message) })
    return () => controller.abort()
  }, [path])

  async function update(endpoint: string, body: unknown, success: string) {
    setBusy(true)
    setError('')
    setNotice('')
    try {
      await request(path + endpoint, {
        method: 'POST',
        headers: { 'Content-Type': 'application/json' },
        body: JSON.stringify(body),
        signal: AbortSignal.timeout(60_000),
      })
      setNotice(success)
      const result = await request<SiteDetails>(path)
      setDetails(result)
      setDomain(result.site.domain ?? '')
      return true
    } catch (reason) {
      setError(reason instanceof Error ? reason.message : 'Request failed. Try again.')
      return false
    } finally {
      setBusy(false)
    }
  }

  async function publish(event: FormEvent<HTMLFormElement>) {
    event.preventDefault()
    const form = event.currentTarget
    const input = form.elements.namedItem('build') as HTMLInputElement
    setBusy(true)
    setError('')
    setNotice('')
    try {
      const files = await encodeBuild(Array.from(input.files ?? []))
      if (await update('/publish', { files }, 'Build published.')) form.reset()
    } catch (reason) {
      setError(reason instanceof Error ? reason.message : 'Could not read the build folder.')
    } finally {
      setBusy(false)
    }
  }

  if (details && details.site.project_id !== projectId) return <p role="alert">Website not found in this project.</p>
  return <section className="grid min-w-0 gap-4">
    <Button asChild variant="outline" className="w-fit"><Link to="/projects/$projectId" params={{ projectId }}>Back to project</Link></Button>
    {error && <Alert variant="destructive" role="alert"><AlertTitle>Website request failed</AlertTitle><AlertDescription>{error}</AlertDescription></Alert>}
    {notice && <p role="status" className="text-sm">{notice}</p>}
    {!details && !error && <p role="status">Loading website…</p>}
    {details && <>
      <div>
        <h1 className="break-words text-2xl font-semibold">{details.site.name}</h1>
        <p className="text-sm text-muted-foreground">Static Website</p>
        {details.site.active_release_id ? <Button asChild variant="outline" className="mt-3"><a href={`/api/v1/static-websites/${siteId}/preview/`} target="_blank" rel="noopener noreferrer">Preview</a></Button> : <Button variant="outline" className="mt-3" disabled>Preview</Button>}
        <p className="mt-2 text-xs text-muted-foreground">{details.site.active_release_id ? 'Preview the published HTML and styles. Scripts and forms are disabled in this dashboard preview. Use Open website for the full interactive site.' : 'Publish a build to enable Preview.'}</p>
      </div>
      <Card><CardHeader><CardTitle><h2>Automatic domain</h2></CardTitle><CardDescription>A separate website origin with JavaScript, CSS and client-side navigation enabled. No custom domain is required.</CardDescription></CardHeader><CardContent className="flex flex-col gap-3">{details.site.automatic_url ? <><p className="break-all text-sm">{details.site.automatic_url}</p>{details.site.active_release_id ? <Button asChild variant="outline" className="self-start"><a href={details.site.automatic_url} target="_blank" rel="noopener noreferrer">Open website</a></Button> : <p className="text-sm text-muted-foreground">Publish a build to make this address live.</p>}</> : <p className="text-sm text-muted-foreground">Ask the server administrator to enable automatic domains and the Caddy proxy.</p>}</CardContent></Card>
      <Card>
        <CardHeader>
          <CardTitle><h2>Custom domain</h2></CardTitle>
          <CardDescription>Point the DNS record at the Ferrix server. Caddy issues the certificate after a build is published.</CardDescription>
        </CardHeader>
        <CardContent className="grid gap-4">
          {details.site.domain && details.site.active_release_id && <a className="break-all underline" href={`https://${details.site.domain}/`} target="_blank" rel="noopener noreferrer">Open {details.site.domain}</a>}
          {canSetDomain ? <form className="grid max-w-md gap-3" onSubmit={event => {
            event.preventDefault()
            void update('/domain', { domain: domain.trim() || null }, 'Domain saved.')
          }}>
            <Field>
              <FieldLabel htmlFor="static-domain">DNS hostname</FieldLabel>
              <Input id="static-domain" value={domain} onChange={event => setDomain(event.target.value)} placeholder="www.example.com" maxLength={253} disabled={busy} />
              <FieldDescription>Only use domains you control. Clear the field to disconnect a domain.</FieldDescription>
            </Field>
            <Button className="w-fit" disabled={busy}>Save domain</Button>
          </form> : <p className="text-sm">{details.site.domain ?? 'Ask a system administrator to assign a domain.'}</p>}
        </CardContent>
      </Card>
      {canManage && <Card>
        <CardHeader><CardTitle><h2>Publish build</h2></CardTitle></CardHeader>
        <CardContent>
          <form onSubmit={publish} className="grid max-w-md gap-3">
            <Field>
              <FieldLabel htmlFor="static-build">Build folder</FieldLabel>
              <Input type="file" id="static-build" name="build" {...{ webkitdirectory: '' }} multiple required disabled={busy} />
              <FieldDescription>Select your output folder with index.html. Up to 512 files and 8 MiB per build.</FieldDescription>
            </Field>
            <Button className="w-fit" disabled={busy}>{busy ? 'Working…' : 'Publish build'}</Button>
          </form>
        </CardContent>
      </Card>}
      <Card>
        <CardHeader><CardTitle><h2>Releases</h2></CardTitle><CardDescription>The five most recent builds are kept for rollback.</CardDescription></CardHeader>
        <CardContent>
          {!details.releases.length ? <p className="text-sm text-muted-foreground">No builds published yet.</p> : <ul className="divide-y">
            {details.releases.map(release => <li key={release.id} className="flex flex-wrap items-center justify-between gap-3 py-3">
              <div className="text-sm">
                <time dateTime={release.created_at}>{new Date(release.created_at).toLocaleString()}</time>
                <p className="text-muted-foreground">{Math.ceil(release.bytes / 1024)} KiB · {release.id.slice(0, 8)}</p>
              </div>
              {details.site.active_release_id === release.id ? <Badge>Live</Badge> : canManage && <Button variant="outline" disabled={busy} onClick={() => void update('/rollback', { release_id: release.id }, 'Release activated.')}>Roll back</Button>}
            </li>)}
          </ul>}
        </CardContent>
      </Card>
    </>}
  </section>
}
