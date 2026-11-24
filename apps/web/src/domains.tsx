import { useEffect, useState } from 'react'
import type { FormEvent } from 'react'
import { ArrowUpRight, RefreshCw, Trash2 } from 'lucide-react'
import { request } from '@/lib/api'
import { Alert, AlertDescription, AlertTitle } from '@/components/ui/alert'
import { Badge } from '@/components/ui/badge'
import { Button } from '@/components/ui/button'
import {
  Card,
  CardAction,
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
import { NativeSelect, NativeSelectOption } from '@/components/ui/native-select'
import { Checkbox } from '@/components/ui/checkbox'
import { Input } from '@/components/ui/input'
import { Skeleton } from '@/components/ui/skeleton'

type Domain = {
  id: string
  hostname: string
  route_kind: 'direct'|'lan'|'tunnel'
  canonical: boolean
  http_status: string
  tunnel_status: string
  verification_token: string
  verified_at: string | null
  dns_status: 'pending' | 'ready' | 'mismatch' | 'unavailable'
  tls_status: 'pending' | 'active' | 'unavailable' | 'disabled'
  certificate_expires_at: string | null
  checked_at: string | null
  message: string | null
}
type Domains = {
  private?:boolean
  diagnostic_url?:string|null
  dashboard_origin?:string
  domains: Domain[]
  automatic_url: string | null
  target_ip: string | null
  https_enabled: boolean
  local: boolean
  port: number | null
}
const dnsLabels = {
  pending: 'Waiting for DNS',
  ready: 'DNS ready',
  mismatch: 'DNS needs attention',
  unavailable: 'DNS unavailable',
}
const tlsLabels = {
  pending: 'Waiting for HTTPS',
  active: 'HTTPS active',
  unavailable: 'HTTPS not verified',
  disabled: 'HTTPS disabled',
}

export function ServiceDomains({
  serviceId,
  canManage,
}: {
  serviceId: string
  canManage: boolean
}) {
  const [data, setData] = useState<Domains | null>(null)
  const [error, setError] = useState('')
  const [notice, setNotice] = useState('')
  const [config,setConfig]=useState('')
  const [canonical,setCanonical]=useState(false)
  const [busy, setBusy] = useState(false)
  const [removing, setRemoving] = useState<Domain | null>(null)
  const base = `/services/${serviceId}/domains`
  useEffect(() => {
    const controller = new AbortController()
    request<Domains>(base, { signal: controller.signal })
      .then(setData)
      .catch((e) => {
        if (!controller.signal.aborted)
          setError(e instanceof Error ? e.message : 'Could not load domains.')
      })
    return () => controller.abort()
  }, [base])
  async function action(work: () => Promise<void>) {
    setBusy(true)
    setError('')
    setNotice('')
    try {
      await work()
      setData(await request<Domains>(base))
    } catch (e) {
      setError(e instanceof Error ? e.message : 'Could not update domains.')
    } finally {
      setBusy(false)
    }
  }
  async function add(event: FormEvent<HTMLFormElement>) {
    event.preventDefault()
    const form = event.currentTarget
    const fields=new FormData(form)
    const hostname = String(fields.get('hostname') ?? '')
    const route_kind=String(fields.get('route_kind')??'direct')
    await action(async () => {
      await request(base, {
        method: 'POST',
        headers: { 'Content-Type': 'application/json' },
        body: JSON.stringify({ hostname, route_kind, canonical }),
      })
      form.reset()
      setNotice(
        'Domain added. Check the DNS records below, then select Check DNS & HTTPS.',
      )
    })
  }
  return (
    <div className="flex flex-col gap-4">
      <Dialog open={!!config} onOpenChange={open=>{if(!open)setConfig('')}}><DialogContent><DialogHeader><DialogTitle>Cloudflare Tunnel rule</DialogTitle><DialogDescription>Merge this single rule before the existing catch-all. Preserve other routes. Supply credentials to cloudflared directly and mount only Caddy’s public CA certificate. Keep origin TLS verification enabled.</DialogDescription></DialogHeader><pre className="max-h-80 overflow-auto rounded-md bg-muted p-3 text-xs">{config}</pre></DialogContent></Dialog>
      {data?.diagnostic_url?<p className="break-all text-xs text-muted-foreground">Loopback diagnostic only: {data.diagnostic_url}. This address works on the Ferrix server; use a domain URL from other devices.</p>:null}
      {error ? (
        <Alert variant="destructive">
          <AlertTitle>Unable to continue</AlertTitle>
          <AlertDescription>
            {error}
            <Button
              variant="outline"
              size="sm"
              disabled={busy}
              onClick={() => void action(async () => {})}
            >
              Retry
            </Button>
          </AlertDescription>
        </Alert>
      ) : null}
      {notice ? (
        <p role="status" className="text-sm">
          {notice}
        </p>
      ) : null}
      {!data && !error ? <Skeleton className="h-48 w-full" /> : null}
      {data ? (
        <>
          {data.automatic_url ? (
            <Card>
              <CardHeader>
                <CardTitle>
                  <h2>Automatic domain</h2>
                </CardTitle>
                <CardDescription>
                  This address follows the active deployment.
                </CardDescription>
              </CardHeader>
              <CardContent>
                <Button asChild variant="link" className="max-w-full">
                  <a
                    href={data.automatic_url}
                    target="_blank"
                    rel="noopener noreferrer"
                  >
                    <span className="truncate">{data.automatic_url}</span>
                    <ArrowUpRight data-icon="inline-end" />
                  </a>
                </Button>
              </CardContent>
            </Card>
          ) : null}
          <Card>
            <CardHeader>
              <CardTitle>
                <h2>Custom domains</h2>
              </CardTitle>
              <CardDescription>
                {data.local
                  ? 'Local domains use .localhost and the Caddy local certificate authority.'
                  : 'Connect your domain to this application. Caddy obtains and renews HTTPS certificates automatically.'}
              </CardDescription>
            </CardHeader>
            <CardContent>
              {!data.target_ip ? (
                <p className="text-sm text-muted-foreground">
                  Configure automatic domains and the Caddy ingress network to
                  connect a custom domain.
                </p>
              ) : canManage ? (
                <form
                  onSubmit={add}
                  className="flex flex-col gap-4 sm:flex-row sm:items-end"
                >
                  <FieldGroup className="flex-1">
                    <Field>
                      <FieldLabel htmlFor="custom-domain">
                        Domain name
                      </FieldLabel>
                      <Input
                        id="custom-domain"
                        name="hostname"
                        placeholder={
                          data.local ? 'myapp.localhost' : 'app.example.com'
                        }
                        required
                        maxLength={253}
                        autoCapitalize="none"
                        autoCorrect="off"
                        spellCheck={false}
                        disabled={busy || data.domains.length >= 10}
                      />
                      <FieldDescription>
                        Enter only the hostname. Up to 10 domains per
                        application.
                      </FieldDescription>
                    </Field>
                  {data.private?<Field><FieldLabel htmlFor="domain-route-kind">Route</FieldLabel><NativeSelect name="route_kind" id="domain-route-kind" defaultValue="tunnel"><NativeSelectOption value="tunnel">Cloudflare Tunnel · public HTTPS</NativeSelectOption><NativeSelectOption value="lan">LAN · split DNS</NativeSelectOption></NativeSelect></Field>:null}
                  <Field orientation="horizontal"><Checkbox id="canonical-domain" checked={canonical} onCheckedChange={v=>setCanonical(v===true)}/><FieldLabel htmlFor="canonical-domain">Preferred website URL after verification</FieldLabel></Field>
                  </FieldGroup>
                  <Button disabled={busy || data.domains.length >= 10}>
                    Add domain
                  </Button>
                </form>
              ) : (
                <p className="text-sm text-muted-foreground">
                  Organization owners and admins can manage domains.
                </p>
              )}
            </CardContent>
          </Card>
          {data.domains.map((domain) => {
            const scheme = domain.route_kind==='tunnel'||data.https_enabled ? 'https' : 'http'
            const port =
              domain.route_kind!=='tunnel' && data.port && data.port !== (data.https_enabled ? 443 : 80)
                ? `:${data.port}`
                : ''
            const url = `${scheme}://${domain.hostname}${port}/`
            return (
              <Card key={domain.id}>
                <CardHeader>
                  <CardTitle className="break-all">
                    <h3>{domain.hostname}</h3>
                  </CardTitle>
                  <CardDescription className="flex flex-wrap gap-2">
                    <Badge
                      variant={
                        domain.dns_status === 'ready' ? 'secondary' : 'outline'
                      }
                    >
                      {dnsLabels[domain.dns_status]}
                    </Badge>
                    <Badge
                      variant={
                        domain.tls_status === 'active' ? 'secondary' : 'outline'
                      }
                    >
                      {domain.tls_status === 'active' &&
                      domain.certificate_expires_at &&
                      new Date(domain.certificate_expires_at).getTime() <=
                        Date.now()
                        ? 'Certificate expired'
                        : tlsLabels[domain.tls_status]}
                    </Badge>
                  </CardDescription>
                  {canManage ? (
                    <CardAction>
                      <Button
                        variant="ghost"
                        size="icon"
                        aria-label={`Remove ${domain.hostname}`}
                        disabled={busy}
                        onClick={() => setRemoving(domain)}
                      >
                        <Trash2 />
                      </Button>
                    </CardAction>
                  ) : null}
                </CardHeader>
                <CardContent className="flex flex-col gap-4">
                  {domain.route_kind==='tunnel'?<><p className="text-sm text-muted-foreground">Configure a Cloudflare published hostname for this application, then publish the TXT ownership record below. Public DNS can remain proxied; no public server IPv4 or inbound port is required.</p><p className="break-all text-xs">TXT: _ferrix-challenge.{domain.hostname} → {domain.verification_token}</p>{canManage?<Button variant="outline" onClick={()=>void action(async()=>{const result=await request<{ingress_rule:unknown}>(`${base}/${domain.id}/tunnel-config`);setConfig(JSON.stringify(result.ingress_rule,null,2))})}>Review tunnel rule</Button>:null}</>:data.local ? (
                    <p className="text-sm text-muted-foreground">
                      This hostname resolves locally. Select Check DNS & HTTPS
                      to activate it. Local HTTPS requires trusting the Caddy
                      root certificate.
                    </p>
                  ) : (
                    <>
                      <p className="text-sm text-muted-foreground">
                        Add these records at your DNS provider. Use DNS-only
                        mode for direct routing. LAN mappings use your internal DNS resolver. Remove conflicting A and AAAA
                        records.
                      </p>
                      <dl className="grid gap-3 text-sm sm:grid-cols-[auto_1fr]">
                        <dt>A record</dt>
                        <dd className="break-all font-mono">
                          {domain.hostname} → {data.target_ip}
                        </dd>
                        <dt>TXT name</dt>
                        <dd className="break-all font-mono">
                          _ferrix-challenge.{domain.hostname}
                        </dd>
                        <dt>TXT value</dt>
                        <dd className="break-all font-mono select-all">
                          {domain.verification_token}
                        </dd>
                      </dl>
                    </>
                  )}
                  {domain.route_kind!=='direct'?<div className="flex flex-wrap gap-2"><Badge variant="outline">{domain.route_kind==='tunnel'?'Tunnel connector: not queried':'LAN route'}</Badge><Badge variant="outline">HTTP: {domain.http_status}</Badge>{domain.canonical?<Badge variant="secondary">Preferred URL</Badge>:null}</div>:null}
                  {domain.message ? (
                    <p className="text-sm" role="status">
                      {domain.message}
                    </p>
                  ) : null}
                  {domain.certificate_expires_at ? (
                    <p className="text-sm text-muted-foreground">
                      Certificate expires{' '}
                      <time dateTime={domain.certificate_expires_at}>
                        {new Date(
                          domain.certificate_expires_at,
                        ).toLocaleString()}
                      </time>
                    </p>
                  ) : null}
                  {domain.checked_at ? (
                    <p className="text-xs text-muted-foreground">
                      Last checked{' '}
                      {new Date(domain.checked_at).toLocaleString()}. Checks are
                      available every 60 seconds.
                    </p>
                  ) : null}
                  <div className="flex flex-wrap gap-2">
                    {canManage&&domain.verified_at&&domain.dns_status==='ready'&&!domain.canonical?<Button variant="outline" disabled={busy} onClick={()=>void action(async()=>{await request(`${base}/${domain.id}/preferred`,{method:'POST'});setNotice('Preferred website URL updated.')})}>Use as preferred URL</Button>:null}
                    {canManage ? (
                      <Button
                        variant="outline"
                        disabled={busy}
                        onClick={() =>
                          void action(async () => {
                            await request(`${base}/${domain.id}/check`, {
                              method: 'POST',
                              signal: AbortSignal.timeout(40_000),
                            })
                            setNotice('Domain check completed.')
                          })
                        }
                      >
                        <RefreshCw data-icon="inline-start" />
                        {busy ? 'Working…' : 'Check DNS & HTTPS'}
                      </Button>
                    ) : null}
                    {domain.dns_status === 'ready' && domain.verified_at ? (
                      <Button asChild variant="outline">
                        <a href={url} target="_blank" rel="noopener noreferrer">
                          Open website
                          <ArrowUpRight data-icon="inline-end" />
                        </a>
                      </Button>
                    ) : null}
                  </div>
                </CardContent>
              </Card>
            )
          })}
        </>
      ) : null}
      <Dialog
        open={removing !== null}
        onOpenChange={(open) => {
          if (!open && !busy) setRemoving(null)
        }}
      >
        <DialogContent>
          <DialogHeader>
            <DialogTitle>Remove domain?</DialogTitle>
            <DialogDescription>
              Requests to {removing?.hostname} will stop reaching this
              application through Ferrix.
            </DialogDescription>
          </DialogHeader>
          <DialogFooter>
            <Button
              variant="outline"
              disabled={busy}
              onClick={() => setRemoving(null)}
            >
              Cancel
            </Button>
            <Button
              variant="destructive"
              disabled={busy}
              onClick={() =>
                void action(async () => {
                  if (!removing) return
                  await request(`${base}/${removing.id}`, { method: 'DELETE' })
                  setRemoving(null)
                  setNotice('Domain removed.')
                })
              }
            >
              Remove domain
            </Button>
          </DialogFooter>
          {error ? (
            <p role="alert" className="text-sm text-destructive">
              {error}
            </p>
          ) : null}
        </DialogContent>
      </Dialog>
    </div>
  )
}
