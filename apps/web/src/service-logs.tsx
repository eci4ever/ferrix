import { useEffect, useRef, useState } from 'react'
import { RefreshCw, Search } from 'lucide-react'
import { ApiRequestError, request } from '@/lib/api'
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
import { ScrollArea } from '@/components/ui/scroll-area'
import { Switch } from '@/components/ui/switch'
import { Field, FieldLabel } from '@/components/ui/field'
import {
  InputGroup,
  InputGroupAddon,
  InputGroupInput,
} from '@/components/ui/input-group'
import { NativeSelect, NativeSelectOption } from '@/components/ui/native-select'

type Logs = {
  logs: string
  truncated?: boolean
  container_id?: string | null
  checked_at?: string
}
export function ServiceLogs({
  serviceId,
  onClose,
}: {
  serviceId: string
  onClose: () => void
}) {
  const [data, setData] = useState<Logs | null>(null)
  const [error, setError] = useState('')
  const [query, setQuery] = useState('')
  const [tail, setTail] = useState('500')
  const [live, setLive] = useState(false)
  const [autoScroll, setAutoScroll] = useState(true)
  const [busy, setBusy] = useState(false)
  const [version, setVersion] = useState(0)
  const [containerChanged, setContainerChanged] = useState(false)
  const loadedSource = useRef('')
  const previousContainer = useRef<string | null>(null)
  const viewport = useRef<HTMLDivElement>(null)
  useEffect(() => {
    const controller = new AbortController()
    let inFlight = false
    const source = `${serviceId}:${tail}:${version}`
    async function refresh() {
      if (document.hidden || inFlight) return
      inFlight = true
      setBusy(true)
      try {
        const result = await request<Logs>(
          `/services/${serviceId}/logs?tail=${tail}&timestamps=true`,
          {
            signal: AbortSignal.any([
              controller.signal,
              AbortSignal.timeout(10_000),
            ]),
          },
        )
        if (controller.signal.aborted) return
        if (
          result.container_id &&
          previousContainer.current &&
          result.container_id !== previousContainer.current
        )
          setContainerChanged(true)
        previousContainer.current = result.container_id ?? null
        setData({
          ...result,
          checked_at: result.checked_at ?? new Date().toISOString(),
        })
        loadedSource.current = source
        setError('')
      } catch (e) {
        if (controller.signal.aborted) return
        if (
          e instanceof ApiRequestError &&
          [401, 403, 404].includes(e.status)
        ) {
          setData(null)
          setLive(false)
          previousContainer.current = null
        }
        if (!controller.signal.aborted)
          setError(
            e instanceof Error ? e.message : 'Could not read container logs.',
          )
      } finally {
        inFlight = false
        if (!controller.signal.aborted) setBusy(false)
      }
    }
    setBusy(false)
    if (live || loadedSource.current !== source) void refresh()
    const timer = live
      ? window.setInterval(() => void refresh(), 2000)
      : undefined
    const visible = () => {
      if (!document.hidden && (live || loadedSource.current !== source))
        void refresh()
    }
    document.addEventListener('visibilitychange', visible)
    return () => {
      controller.abort()
      window.clearInterval(timer)
      document.removeEventListener('visibilitychange', visible)
    }
  }, [serviceId, tail, live, version])
  useEffect(() => {
    if (autoScroll && !query && viewport.current)
      viewport.current.scrollTop = viewport.current.scrollHeight
  }, [data, autoScroll, query])
  // Logs remain plain text. Strip ANSI styling to keep output readable in a browser.
  const text = data?.logs.replace(/\u001b\[[0-?]*[ -/]*[@-~]/g, '') ?? ''
  const lines = text.split('\n')
  const needle = query.toLocaleLowerCase()
  const matching = query
    ? lines.filter((line) => line.toLocaleLowerCase().includes(needle))
    : lines
  const filtered = matching.join('\n')
  return (
    <Card>
      <CardHeader>
        <CardTitle>
          <h2>Container logs</h2>
        </CardTitle>
        <CardDescription>
          Latest {tail} lines, up to 64 KiB. Logs may contain application
          secrets. Search applies to this visible buffer.
        </CardDescription>
        <CardAction>
          <Button variant="ghost" onClick={onClose}>
            Close
          </Button>
        </CardAction>
      </CardHeader>
      <CardContent className="flex flex-col gap-4">
        {error ? (
          <Alert variant="destructive">
            <AlertTitle>Log refresh failed</AlertTitle>
            <AlertDescription>
              {error}
              {data ? ' Previous output is retained.' : ''}
            </AlertDescription>
          </Alert>
        ) : null}
        <div className="flex flex-wrap items-end gap-3">
          <Field className="min-w-0 flex-1">
            <FieldLabel htmlFor="service-log-search">Search logs</FieldLabel>
            <InputGroup>
              <InputGroupInput
                id="service-log-search"
                placeholder="Find text in logs…"
                value={query}
                onChange={(event) => setQuery(event.target.value)}
                autoComplete="off"
                spellCheck={false}
              />
              <InputGroupAddon>
                <Search />
              </InputGroupAddon>
            </InputGroup>
          </Field>
          <Field className="w-36">
            <FieldLabel htmlFor="service-log-lines">Log lines</FieldLabel>
            <NativeSelect
              id="service-log-lines"
              value={tail}
              onChange={(event) => setTail(event.target.value)}
            >
              <NativeSelectOption value="100">100 lines</NativeSelectOption>
              <NativeSelectOption value="500">500 lines</NativeSelectOption>
              <NativeSelectOption value="1000">1000 lines</NativeSelectOption>
            </NativeSelect>
          </Field>
        </div>
        <div className="flex flex-wrap items-center gap-3">
          <Field orientation="horizontal" className="w-auto"><Switch id="service-log-live" checked={live} onCheckedChange={setLive} /><FieldLabel htmlFor="service-log-live">Live tail</FieldLabel></Field>
          <Button
            variant="outline"
            disabled={busy}
            onClick={() => setVersion((value) => value + 1)}
          >
            <RefreshCw data-icon="inline-start" />
            Refresh logs
          </Button>
          <label className="flex cursor-pointer items-center gap-2 text-sm">
            <Switch
              checked={autoScroll}
              onCheckedChange={setAutoScroll}
            />
            Auto-scroll
          </label>
          {query ? (
            <Button variant="ghost" onClick={() => setQuery('')}>
              Clear search
            </Button>
          ) : null}
          <Badge variant="outline">{live ? 'Live · 2s' : 'Paused'}</Badge>
        </div>
        {containerChanged ? (
          <p role="status" className="text-sm text-muted-foreground">
            Container changed. Showing the active deployment’s logs.
          </p>
        ) : null}
        {data?.truncated ? (
          <p role="status" className="text-sm text-muted-foreground">
            Output exceeds the buffer limit. Showing the newest 64 KiB.
          </p>
        ) : null}
        <ScrollArea className="h-[min(32rem,60dvh)] min-h-32 min-w-0 rounded-md border bg-muted/30" viewportProps={{ ref: viewport, role: 'region', 'aria-label': 'Container log output', 'aria-busy': busy, tabIndex: 0 }}>
          <pre className="min-w-0 whitespace-pre-wrap break-all p-4 text-xs">
          <code>
            {data === null
              ? error
                ? 'Logs unavailable.'
                : 'Loading logs…'
              : filtered ||
                (query ? 'No matching log lines.' : 'No output yet.')}
          </code>
          </pre>
        </ScrollArea>
        <div className="flex flex-wrap gap-2 text-xs text-muted-foreground">
          {query ? (
            <span>{matching.filter(Boolean).length} matching lines</span>
          ) : null}
          {data?.checked_at ? (
            <span>
              Last updated {new Date(data.checked_at).toLocaleTimeString()}
            </span>
          ) : null}
          <span>
            {live
              ? 'Live tail pauses while the page is hidden.'
              : 'Live tail is paused.'}
          </span>
        </div>
      </CardContent>
    </Card>
  )
}
