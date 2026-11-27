import { useId } from 'react'
import { Area, AreaChart, CartesianGrid, ReferenceLine, XAxis, YAxis, Pie, PieChart } from 'recharts'
import { ChartContainer, ChartTooltip, ChartTooltipContent, ChartLegend, ChartLegendContent } from '@/components/ui/chart'
import type { ChartDatum } from '@/lib/monitoring-chart-data'

export type MonitoringSeries = { key: string; name: string; color: string; dash?: string }
export function MonitoringChart({ rows, series, label, domain, maximum, format, markers = [] }: { rows: ChartDatum[]; series: MonitoringSeries[]; label: string; domain: [number, number]; maximum?: number; format: (value: number | null) => string; markers?: { id: string; at: number; label: string }[] }) {
  const id = useId().replace(/:/g, '')
  const config = Object.fromEntries(series.map(item => [item.key, { label: item.name, color: item.color }]))
  return <><ChartContainer config={config} className="mt-4 h-56 w-full min-w-0 aspect-auto" role="group" aria-label={label}>
    <AreaChart accessibilityLayer data={rows} margin={{ left: 0, right: 12, top: 12, bottom: 0 }}>
      <defs>{series.map(item => <linearGradient key={item.key} id={`${id}-${item.key}`} x1="0" y1="0" x2="0" y2="1"><stop offset="0%" stopColor={item.color} stopOpacity={0.4} /><stop offset="95%" stopColor={item.color} stopOpacity={0.025} /></linearGradient>)}</defs>
      <CartesianGrid vertical={false} stroke="var(--border)" strokeDasharray="3 3" />
      <XAxis dataKey="at" type="number" scale="time" domain={domain} allowDataOverflow tickFormatter={value => new Date(value).toLocaleTimeString([], { hour: '2-digit', minute: '2-digit' })} tickLine={false} axisLine={false} minTickGap={36} />
      <YAxis domain={[0, maximum ?? 'auto']} tickFormatter={value => format(Number(value))} width={72} tickLine={false} axisLine={false} tickCount={4} />
      <ChartTooltip filterNull={false} content={<ChartTooltipContent labelFormatter={(_label, payload) => { const at = payload?.[0]?.payload?.at; return typeof at === 'number' ? new Date(at).toLocaleString() : 'Sample time unavailable' }} formatter={(value, name) => <div className="flex w-full items-center justify-between gap-4"><span className="text-muted-foreground">{config[String(name)]?.label ?? String(name)}</span><span className="font-mono font-medium tabular-nums">{format(typeof value === 'number' ? value : null)}</span></div>} />} />
      {markers.map(marker => <ReferenceLine key={marker.id} x={marker.at} stroke="var(--muted-foreground)" strokeDasharray="2 5" label={{ value: '↻', position: 'insideTop', fill: 'var(--muted-foreground)' }} />)}
      {series.map(item => <Area key={item.key} name={item.key} dataKey={item.key} type="linear" connectNulls={false} stroke={item.color} strokeWidth={2} strokeDasharray={item.dash} fill={`url(#${id}-${item.key})`} dot={false} activeDot={{ r: 3 }} isAnimationActive={false} />)}
      <ChartLegend content={<ChartLegendContent />} />
    </AreaChart>
  </ChartContainer>{markers.length ? <ul className="sr-only" aria-label="Chart change markers">{markers.map(marker => <li key={marker.id}>{new Date(marker.at).toLocaleString()}: {marker.label}</li>)}</ul> : null}</>
}
export function StorageChart({ parts, format }: { parts: { name: string; value: number | null; color: string }[]; format: (value: number | null) => string }) {
  const positive = parts.filter(part => part.value !== null && part.value > 0).map(part => ({ ...part, fill: part.color }))
  return <ChartContainer config={{}} className="mx-auto h-52 w-52 aspect-square" role="group" aria-label="Docker storage breakdown">
    <PieChart accessibilityLayer><Pie data={positive.length ? positive : [{ name: 'No measured disk usage', value: 1, fill: 'var(--muted)' }]} dataKey="value" nameKey="name" innerRadius={72} outerRadius={92} paddingAngle={positive.length > 1 ? 2 : 0} stroke="none" isAnimationActive={false} />{positive.length ? <ChartTooltip content={<ChartTooltipContent hideLabel formatter={(value, name) => <span>{name}: {format(typeof value === 'number' ? value : null)}</span>} />} /> : null}</PieChart>
  </ChartContainer>
}
