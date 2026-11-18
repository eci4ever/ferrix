import { useEffect, useId, useState } from 'react'
import { Card, CardContent, CardDescription, CardHeader, CardTitle } from '@/components/ui/card'
import { Alert, AlertDescription, AlertTitle } from '@/components/ui/alert'
import { Badge } from '@/components/ui/badge'
import { Button } from '@/components/ui/button'
import { Skeleton } from '@/components/ui/skeleton'
import { request } from '@/lib/api'

type Sample = { at: number; cpu: number | null; memory_used: number; memory_total: number; disk_used: number | null; disk_total: number | null; read_bps: number | null; write_bps: number | null; receive_bps: number | null; transmit_bps: number | null }
type Storage = { at: number | null; images: number | null; containers: number | null; volumes: number | null; build_cache: number | null; error: string | null }
type Snapshot = { host: string; os: string; scope: string; interval_seconds: number; samples: Sample[]; docker: Storage }
type Series = { name: string; key: keyof Omit<Sample, 'at'>; color: string }
const colors = ['var(--chart-1)', 'var(--chart-2)', 'var(--chart-3)', 'var(--chart-4)']
function bytes(n: number | null | undefined): string {
  if (n == null) return 'Unavailable'
  if (n === 0) return '0 B'
  const i = Math.min(4, Math.max(0, Math.floor(Math.log(n) / Math.log(1024))))
  return `${(n / 1024 ** i).toLocaleString(undefined, { maximumFractionDigits: 2 })} ${['B', 'KiB', 'MiB', 'GiB', 'TiB'][i]}`
}
const percent = (n: number | null | undefined) => n == null ? 'Unavailable' : `${n.toFixed(1)}%`
const rate = (n: number | null | undefined) => n == null ? 'Unavailable' : `${bytes(n)}/s`
const time = (n: number) => new Date(n).toLocaleTimeString([], { hour: '2-digit', minute: '2-digit', second: '2-digit' })

function Meter({ value }: { value: number | null }) {
  return <div role="meter" aria-label="Current usage" aria-valuemin={0} aria-valuemax={100} aria-valuenow={value ?? undefined} aria-valuetext={value == null ? 'Unavailable' : percent(value)} className="h-2 overflow-hidden rounded-full bg-muted"><div className="h-full rounded-full bg-primary" style={{ width: `${Math.max(0, Math.min(100, value ?? 0))}%` }} /></div>
}
function History({ samples, series, max, format = bytes }: { samples: Sample[]; series: Series[]; max?: number; format?: (n: number | null) => string }) {
  const id = useId()
  const end = samples.at(-1)?.at ?? Date.now()
  const start = end - 15 * 60 * 1000
  const ceiling = Math.max(1, max ?? Math.max(...samples.flatMap(s => series.map(line => s[line.key] ?? 0))) * 1.15)
  const x = (at: number) => 66 + (at - start) / (end - start) * 614
  const y = (v: number) => 158 - Math.min(1, v / ceiling) * 140
  return <>
    <svg viewBox="0 0 700 195" className="mt-4 w-full overflow-visible" role="img" aria-labelledby={id}>
      <title id={id}>{series.map(s => s.name).join(' and ')} over the last 15 minutes. Exact readings are available below.</title>
      {[0, .5, 1].map(v => <g key={v}><line x1="66" x2="680" y1={y(v * ceiling)} y2={y(v * ceiling)} stroke="var(--border)" /><text x="58" y={y(v * ceiling) + 4} textAnchor="end" fontSize="10" fill="var(--muted-foreground)">{format(v * ceiling)}</text></g>)}
      {series.map(line => {
        let path = ''; let connected = false
        samples.forEach(s => { const value = s[line.key]; if (value == null) { connected = false; return }; path += `${connected ? 'L' : 'M'}${x(s.at)},${y(value)} `; connected = true })
        const last = samples.at(-1)
        return <g key={line.key}><path d={path} fill="none" stroke={line.color} strokeWidth="2" vectorEffect="non-scaling-stroke" />{last && last[line.key] != null ? <circle cx={x(last.at)} cy={y(last[line.key]!)} r="3" fill={line.color} /> : null}</g>
      })}
      <text x="66" y="184" fontSize="11" fill="var(--muted-foreground)">{time(start)}</text><text x="680" y="184" textAnchor="end" fontSize="11" fill="var(--muted-foreground)">{time(end)}</text>
    </svg>
    <div className="flex flex-wrap justify-center gap-4 text-xs text-muted-foreground">{series.map(s => <span key={s.key} className="flex items-center gap-2"><span className="size-2 rounded-sm" style={{ background: s.color }} />{s.name}</span>)}</div>
    <details className="mt-4 text-xs text-muted-foreground"><summary className="w-fit cursor-pointer rounded-sm focus-visible:outline-2">View exact readings</summary><div className="mt-2 max-h-48 overflow-auto"><table className="w-full text-left tabular-nums"><caption className="sr-only">Recorded readings</caption><thead><tr><th className="p-1">Time</th>{series.map(s => <th className="p-1" key={s.key}>{s.name}</th>)}</tr></thead><tbody>{samples.map(s => <tr key={s.at}><td className="p-1">{time(s.at)}</td>{series.map(line => <td className="p-1" key={line.key}>{format(s[line.key])}</td>)}</tr>)}</tbody></table></div></details>
  </>
}
function DockerStorage({ storage }: { storage: Storage }) {
  const parts = [{ name: 'Images', value: storage.images }, { name: 'Containers', value: storage.containers }, { name: 'Volumes', value: storage.volumes }]
  const complete = parts.every(p => p.value != null)
  const total = parts.reduce((sum, p) => sum + (p.value ?? 0), 0)
  let offset = 0
  return <Card><CardHeader><CardTitle>Docker disk usage</CardTitle><CardDescription>Connected Docker daemon · all containers and projects</CardDescription></CardHeader><CardContent>
    {storage.error ? <p role="status" className="mb-3 text-sm text-destructive">{storage.error}</p> : null}
    <div className="relative mx-auto w-52"><svg viewBox="0 0 200 200" className="w-full" role="img" aria-label={complete ? `Docker storage: ${bytes(total)}` : 'Docker storage breakdown incomplete'}><circle cx="100" cy="100" r="76" fill="none" stroke="var(--muted)" strokeWidth="22" />{parts.map((p, i) => { const length = total ? (p.value ?? 0) / total * 100 : 0; const previous = offset; offset += length; return <circle key={p.name} cx="100" cy="100" r="76" pathLength="100" fill="none" stroke={colors[i]} strokeWidth="22" strokeDasharray={`${length} ${100 - length}`} strokeDashoffset={-previous} transform="rotate(-90 100 100)" /> })}</svg><div className="pointer-events-none absolute inset-0 flex flex-col items-center justify-center"><strong className="text-xl tabular-nums">{storage.at ? bytes(total) : 'Waiting…'}</strong><span className="text-xs text-muted-foreground">{complete ? 'Docker usage' : 'Known usage only'}</span></div></div>
    <dl className="mt-4 grid gap-2 text-sm">{parts.map((p, i) => <div className="flex justify-between gap-2" key={p.name}><dt className="flex items-center gap-2"><span className="size-2 rounded-sm" style={{ background: colors[i] }} />{p.name}</dt><dd className="tabular-nums">{bytes(p.value)}</dd></div>)}</dl>
    <p className="mt-4 text-xs text-muted-foreground">Build cache: {bytes(storage.build_cache)}. Separate from total; may share image layers. Docker object storage, not VM disk capacity.</p>
    <p className="mt-2 text-xs text-muted-foreground">{storage.at ? `Updated ${time(storage.at)} · cached for 60 seconds` : 'Waiting for Docker storage probe…'}</p>
  </CardContent></Card>
}
export function MonitoringPage({ admin }: { admin: boolean }) {
  const [data, setData] = useState<Snapshot | null>(null)
  const [error, setError] = useState('')
  const [retry, setRetry] = useState(0)
  const [now, setNow] = useState(Date.now())
  useEffect(() => {
    if (!admin) return
    let stopped = false; let timer: ReturnType<typeof setTimeout>; let controller: AbortController | undefined
    async function poll() {
      if (stopped) return
      if (!document.hidden) {
        controller = new AbortController()
        const timeout = setTimeout(() => controller?.abort(), 8000)
        try { const next = await request<Snapshot>('/monitoring', { signal: controller.signal }); if (!stopped) { setData(next); setError(''); setNow(Date.now()) } }
        catch (e) { if (!stopped) { setError(e instanceof Error ? e.message : 'Monitoring unavailable.'); setNow(Date.now()) } }
        finally { clearTimeout(timeout) }
      }
      if (!stopped) timer = setTimeout(poll, 5000)
    }
    void poll()
    return () => { stopped = true; clearTimeout(timer); controller?.abort() }
  }, [admin, retry])
  if (!admin) return <Alert><AlertTitle>Administrator access required</AlertTitle><AlertDescription>Host monitoring is available only to Ferrix system administrators.</AlertDescription></Alert>
  const samples = data?.samples ?? []
  const latest = samples.at(-1)
  const stale = !!latest && now - latest.at > 20_000
  const fraction = (a: number | null | undefined, b: number | null | undefined) => a != null && b ? a / b * 100 : null
  return <>
    <div className="flex flex-wrap items-start justify-between gap-3"><div><h1 className="text-2xl font-semibold tracking-tight">Monitoring</h1><p className="mt-1 text-sm text-muted-foreground">Watch your server’s CPU, memory, storage and I/O.</p></div><Badge variant="outline">Admin · host scope</Badge></div>
    {error ? <Alert variant="destructive"><AlertTitle>Monitoring refresh failed</AlertTitle><AlertDescription>{error} {data ? 'Previous readings remain visible.' : ''}<Button variant="outline" size="sm" onClick={() => setRetry(v => v + 1)}>Retry</Button></AlertDescription></Alert> : null}
    {data ? <><div className="text-sm text-muted-foreground"><p><span className="font-medium text-foreground">{data.host}</span> · {data.os} · {latest ? `Sampled ${time(latest.at)}` : 'Warming up…'} · every {data.interval_seconds}s</p><p className="mt-1">API host metrics. In containers, CPU and memory reflect the visible OS, not container limits. Docker storage comes from the connected daemon, which may run in a VM.</p>{stale ? <p role="status" className="mt-2 font-medium text-destructive">Host readings are stale. Collector has not updated for over 20 seconds.</p> : null}</div>
    <div className="grid min-w-0 gap-6 lg:grid-cols-2">
      <Card><CardHeader><CardTitle>CPU usage</CardTitle><CardDescription>Used: {percent(latest?.cpu)} · all logical CPUs, normalized to 100%</CardDescription></CardHeader><CardContent><Meter value={latest?.cpu ?? null} /><History samples={samples} max={100} format={percent} series={[{ name: 'CPU', key: 'cpu', color: colors[0] }]} /></CardContent></Card>
      <Card><CardHeader><CardTitle>Memory usage</CardTitle><CardDescription>Used: {bytes(latest?.memory_used)} / Total: {bytes(latest?.memory_total)}</CardDescription></CardHeader><CardContent><Meter value={fraction(latest?.memory_used, latest?.memory_total)} /><History samples={samples} max={latest?.memory_total} series={[{ name: 'Memory used', key: 'memory_used', color: colors[1] }]} /></CardContent></Card>
      <Card><CardHeader><CardTitle>Disk space</CardTitle><CardDescription>Root filesystem (/) · Used: {bytes(latest?.disk_used)} / Total: {bytes(latest?.disk_total)}</CardDescription></CardHeader><CardContent><Meter value={fraction(latest?.disk_used, latest?.disk_total)} /><History samples={samples} max={latest?.disk_total ?? undefined} series={[{ name: 'Disk used', key: 'disk_used', color: colors[2] }]} /></CardContent></Card>
      <DockerStorage storage={data.docker} />
      <Card><CardHeader><CardTitle>Block I/O</CardTitle><CardDescription>Read: {rate(latest?.read_bps)} · Write: {rate(latest?.write_bps)}</CardDescription></CardHeader><CardContent><History samples={samples} format={rate} series={[{ name: 'Read', key: 'read_bps', color: colors[0] }, { name: 'Write', key: 'write_bps', color: colors[1] }]} /><p className="mt-2 text-xs text-muted-foreground">API host · mounted block devices · bytes per second.</p></CardContent></Card>
      <Card><CardHeader><CardTitle>Network I/O</CardTitle><CardDescription>In: {rate(latest?.receive_bps)} · Out: {rate(latest?.transmit_bps)}</CardDescription></CardHeader><CardContent><History samples={samples} format={rate} series={[{ name: 'Received', key: 'receive_bps', color: colors[0] }, { name: 'Transmitted', key: 'transmit_bps', color: colors[1] }]} /><p className="mt-2 text-xs text-muted-foreground">API host · non-loopback interfaces · bytes per second.</p></CardContent></Card>
    </div><p className="text-xs text-muted-foreground">Last 15 minutes · up to 180 samples · history resets when the API restarts</p></> : !error ? <div role="status" aria-label="Loading monitoring" className="grid gap-6 lg:grid-cols-2">{Array.from({ length: 6 }, (_, i) => <Skeleton key={i} className="h-72 w-full" />)}</div> : null}
  </>
}
