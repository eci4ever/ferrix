export type ChartDatum = { id: string; at: number; [key: string]: string | number | null }
export type RetainedPoint = { id: number; deployment_id: string | null; container_id: string | null; image_id: string | null; collected_at: string; error: boolean; marker: boolean; data: Record<string, number | string | boolean | null> }
export type RetainedHistory = { samples: RetainedPoint[]; sample_seconds: number; downsample_after_minutes: number; downsample_seconds: number; retention_days: number; truncated: boolean; gaps_note: string; scope: string }
export function insertChartGaps<T extends ChartDatum>(rows: T[], keys: string[], breaks: (previous: T, next: T, index: number) => boolean): ChartDatum[] {
  return rows.flatMap((row, index) => {
    const previous = rows[index - 1]
    if (!previous || !breaks(previous, row, index)) return [row]
    return [{ id: `gap-${previous.id}-${row.id}`, at: previous.at + (row.at - previous.at) / 2, ...Object.fromEntries(keys.map(key => [key, null])) }, row]
  })
}
const num = (point: RetainedPoint, key: string) => typeof point.data[key] === 'number' && Number.isFinite(point.data[key]) ? point.data[key] as number : null
export function retainedChartData(history: RetainedHistory, metric: string, now = Date.now()) {
  const rows = history.samples.map((point, index) => {
    const old = history.samples[index - 1]
    const dt = old ? (Date.parse(point.collected_at) - Date.parse(old.collected_at)) / 1000 : 0
    const rate = (key: string) => {
      const current = num(point, key), previous = old ? num(old, key) : null
      return !old?.error && current !== null && previous !== null && current >= previous && dt > 0 && old?.container_id === point.container_id ? (current - previous) / dt : null
    }
    let a: number | null = null, b: number | null = null
    if (!point.error) {
      if (metric === 'percent') {
        a = num(point, 'cpu_percent_of_limit') ?? num(point, 'cpu')
        const used = num(point, 'memory_used'), total = num(point, 'memory_total')
        b = num(point, 'memory_percent_of_limit') ?? (used !== null && total ? used / total * 100 : null)
      } else if (metric === 'network') {
        a = num(point, 'receive_bps') ?? rate('network_rx_bytes'); b = num(point, 'transmit_bps') ?? rate('network_tx_bytes')
      } else {
        a = num(point, 'read_bps') ?? rate('disk_read_bytes'); b = num(point, 'write_bps') ?? rate('disk_write_bytes')
      }
    }
    return { id: String(point.id), at: Date.parse(point.collected_at), a, b }
  })
  const chartRows = insertChartGaps(rows, ['a', 'b'], (previous, next, index) => {
    const point = history.samples[index], old = history.samples[index - 1]
    const cadence = now - next.at > history.downsample_after_minutes * 60_000 ? history.downsample_seconds : history.sample_seconds
    return next.at - previous.at > cadence * 3000 || point.container_id !== old.container_id
  })
  return { rows, chartRows, markers: history.samples.filter(point => point.marker).map(point => ({ id: String(point.id), at: Date.parse(point.collected_at), label: `Deployment / health / restart change${point.deployment_id ? ` · ${point.deployment_id.slice(0, 8)}` : ''}` })) }
}
