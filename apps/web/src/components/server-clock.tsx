import { useEffect, useState } from 'react'
import { Badge } from '@/components/ui/badge'
import { Tooltip, TooltipContent, TooltipTrigger } from '@/components/ui/tooltip'
import { request } from '@/lib/api'

type ClockResponse = { unix_ms: number; timezone: string; utc_offset: string }
type ClockSample = ClockResponse & { receivedAt: number }

export function ServerClock() {
  const [sample, setSample] = useState<ClockSample | null>(null)
  const [tick, setTick] = useState(0)
  useEffect(() => {
    const controller = new AbortController()
    let inFlight = false
    async function sync() {
      if (inFlight || document.hidden) return
      inFlight = true
      try {
        const start = performance.now()
        const clock = await request<ClockResponse>('/server-time', { signal: AbortSignal.any([controller.signal, AbortSignal.timeout(5000)]) })
        const receivedAt = performance.now()
        // Validate the zone before displaying a response. Never fall back to the browser clock.
        new Intl.DateTimeFormat('en-GB', { timeZone: clock.timezone }).format(new Date(clock.unix_ms))
        if (!controller.signal.aborted) {
          setSample({ ...clock, unix_ms: clock.unix_ms + (receivedAt - start) / 2, receivedAt })
          setTick(receivedAt)
        }
      } catch {
        if (!controller.signal.aborted) setSample(null)
      } finally { inFlight = false }
    }
    const resume = () => { if (!document.hidden) void sync() }
    void sync()
    const ticker = window.setInterval(() => setTick(performance.now()), 1000)
    const resync = window.setInterval(() => { void sync() }, 60_000)
    document.addEventListener('visibilitychange', resume)
    return () => {
      controller.abort()
      window.clearInterval(ticker)
      window.clearInterval(resync)
      document.removeEventListener('visibilitychange', resume)
    }
  }, [])
  const time = sample ? new Intl.DateTimeFormat('en-GB', { hour: '2-digit', minute: '2-digit', second: '2-digit', hourCycle: 'h23', timeZone: sample.timezone }).format(new Date(sample.unix_ms + Math.max(0, tick - sample.receivedAt))) : null
  const description = sample ? `Server time · ${sample.timezone} · ${sample.utc_offset}` : 'Server time unavailable'
  return <Tooltip><TooltipTrigger asChild><Badge tabIndex={0} variant="outline" className="gap-2 whitespace-nowrap" aria-label={description}>
    <span><span className="hidden sm:inline">Server Time: </span><span className="tabular-nums">{time ?? 'Unavailable'}</span></span>
    {sample ? <span className="hidden text-muted-foreground xl:inline">{sample.timezone} | {sample.utc_offset}</span> : null}
  </Badge></TooltipTrigger><TooltipContent>{description}</TooltipContent></Tooltip>
}
