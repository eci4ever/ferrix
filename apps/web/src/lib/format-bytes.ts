export function formatBytes(n: number | null | undefined): string {
  if (n == null) return 'Unavailable'
  if (n === 0) return '0 B'
  const i = Math.min(4, Math.max(0, Math.floor(Math.log(n) / Math.log(1024))))
  return `${(n / 1024 ** i).toLocaleString(undefined, { maximumFractionDigits: 2 })} ${['B', 'KiB', 'MiB', 'GiB', 'TiB'][i]}`
}
