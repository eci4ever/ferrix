import { TelemetryHistory } from './telemetry-history'
import { useEffect, useState } from 'react'
import { ApiRequestError, request } from '@/lib/api'
import { Meter, formatBytes } from './monitoring'
import { ServiceStatus } from './service-status'
import { Alert, AlertDescription, AlertTitle } from '@/components/ui/alert'
import { Badge } from '@/components/ui/badge'
import { Button } from '@/components/ui/button'
import {
  Card,
  CardContent,
  CardDescription,
  CardHeader,
  CardTitle,
} from '@/components/ui/card'
import { Skeleton } from '@/components/ui/skeleton'

type Metrics = {
  container_id: string | null
  state: string
  checked_at: string
  cpu_cores_used: number | null
  cpu_limit_cores: number | null
  cpu_percent_of_limit: number | null
  memory_used_bytes: number | null
  memory_raw_bytes: number | null
  memory_limit_bytes: number | null
  memory_percent_of_limit: number | null
}
const percent = (value: number | null) =>
  value === null ? 'Unavailable' : `${value.toFixed(1)}%`
export function ServiceMetrics({ serviceId }: { serviceId: string }) {
  const [data, setData] = useState<Metrics | null>(null)
  const [error, setError] = useState('')
  const [retry, setRetry] = useState(0)
  useEffect(() => {
    const controller = new AbortController()
    let inFlight = false
    async function refresh() {
      if (document.hidden || inFlight) return
      inFlight = true
      try {
        const result = await request<Metrics>(
          `/services/${serviceId}/metrics`,
          {
            signal: AbortSignal.any([
              controller.signal,
              AbortSignal.timeout(12_000),
            ]),
          },
        )
        if (!controller.signal.aborted) {
          setData(result)
          setError('')
        }
      } catch (e) {
        if (controller.signal.aborted) return
        if (e instanceof ApiRequestError && [401, 403, 404].includes(e.status))
          setData(null)
        if (!controller.signal.aborted)
          setError(
            e instanceof Error
              ? e.message
              : 'Could not read service resource usage.',
          )
      } finally {
        inFlight = false
      }
    }
    void refresh()
    const timer = window.setInterval(() => void refresh(), 5000)
    document.addEventListener('visibilitychange', refresh)
    return () => {
      controller.abort()
      window.clearInterval(timer)
      document.removeEventListener('visibilitychange', refresh)
    }
  }, [serviceId, retry])
  return (
    <div className="flex flex-col gap-4">
      <TelemetryHistory serviceId={serviceId}/>
      <Card>
        <CardHeader>
          <CardTitle>
            <h2>Service monitoring</h2>
          </CardTitle>
          <CardDescription>
            Active container usage compared with its configured limits. Updates
            every 5 seconds while this page is visible.
          </CardDescription>
        </CardHeader>
        <CardContent className="flex flex-wrap items-center gap-3">
          {data ? (
            <>
              <ServiceStatus state={data.state} label />
              <p className="text-xs text-muted-foreground">
                Last updated {new Date(data.checked_at).toLocaleTimeString()}
              </p>
            </>
          ) : (
            <Skeleton className="h-5 w-48" />
          )}
          {error ? <Badge variant="outline">Readings unavailable</Badge> : null}
        </CardContent>
      </Card>
      {error ? (
        <Alert variant="destructive">
          <AlertTitle>Unable to refresh usage</AlertTitle>
          <AlertDescription>
            <p>{error}</p>
            {data ? (
              <p>Displayed readings are from the last successful update.</p>
            ) : null}
            <Button
              variant="outline"
              size="sm"
              onClick={() => setRetry((value) => value + 1)}
            >
              Retry
            </Button>
          </AlertDescription>
        </Alert>
      ) : null}
      {data ? (
        <div className="grid gap-4 md:grid-cols-2">
          <Card>
            <CardHeader>
              <CardTitle>
                <h3>CPU</h3>
              </CardTitle>
              <CardDescription>
                Percentage of the service CPU limit
              </CardDescription>
            </CardHeader>
            <CardContent className="flex flex-col gap-4">
              <p className="text-3xl font-semibold tabular-nums">
                {percent(data.cpu_percent_of_limit)}
              </p>
              <Meter
                value={data.cpu_percent_of_limit}
                label="CPU limit usage"
              />
              <p className="text-sm text-muted-foreground">
                {data.cpu_cores_used === null
                  ? 'Usage unavailable'
                  : `${data.cpu_cores_used.toFixed(3)} vCPU used`}{' '}
                ·{' '}
                {data.cpu_limit_cores === null
                  ? 'No explicit CPU limit'
                  : `${data.cpu_limit_cores.toFixed(2)} vCPU limit`}
              </p>
            </CardContent>
          </Card>
          <Card>
            <CardHeader>
              <CardTitle>
                <h3>Memory (RAM)</h3>
              </CardTitle>
              <CardDescription>
                Usage excludes reclaimable file cache
              </CardDescription>
            </CardHeader>
            <CardContent className="flex flex-col gap-4">
              <p className="text-3xl font-semibold tabular-nums">
                {percent(data.memory_percent_of_limit)}
              </p>
              <Meter
                value={data.memory_percent_of_limit}
                label="RAM limit usage"
              />
              <p className="text-sm text-muted-foreground">
                {formatBytes(data.memory_used_bytes)} used ·{' '}
                {data.memory_limit_bytes === null
                  ? 'No explicit memory limit'
                  : `${formatBytes(data.memory_limit_bytes)} limit`}
              </p>
              {data.memory_raw_bytes !== null ? (
                <p className="text-xs text-muted-foreground">
                  Including cache: {formatBytes(data.memory_raw_bytes)}
                </p>
              ) : null}
            </CardContent>
          </Card>
        </div>
      ) : !error ? (
        <div className="grid gap-4 md:grid-cols-2">
          <Skeleton className="h-48 w-full" />
          <Skeleton className="h-48 w-full" />
        </div>
      ) : null}
      {data && data.state !== 'running' ? (
        <p className="text-sm text-muted-foreground">
          Deploy or start this service to collect resource readings.
        </p>
      ) : null}
    </div>
  )
}
