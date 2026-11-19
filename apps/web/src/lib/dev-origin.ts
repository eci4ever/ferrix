const loopbackHosts = new Set(['localhost', '127.0.0.1', '[::1]'])

// Keep local development on the configured authentication origin. This applies
// only to document navigation; API requests retain the backend's Origin checks.
export function localOriginRedirect(authOrigin: string, host: string | undefined, method: string | undefined, accept: string | undefined, target: string | undefined): string | null {
  if (!host || !target?.startsWith('/') || target.startsWith('/api') || !['GET', 'HEAD'].includes(method ?? '') || !accept?.includes('text/html')) return null
  try {
    const canonical = new URL(authOrigin)
    const incoming = new URL(`http://${host}`)
    if (canonical.protocol !== 'http:' || !loopbackHosts.has(canonical.hostname) || !loopbackHosts.has(incoming.hostname)) return null
    if (canonical.origin !== authOrigin || incoming.port !== canonical.port || incoming.host === canonical.host) return null
    return canonical.origin + target
  } catch { return null }
}
