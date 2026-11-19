export type Project = { id: string; name: string; description: string; created_at: string; service_count: number }
export type SystemStatus = { database: boolean; docker: boolean; version: string }
export type Container = { id: string; name: string; image: string; state: string; status: string }

export async function request<T>(path: string, init?: RequestInit): Promise<T> {
  let response: Response
  try {
    response = await fetch(`/api/v1${path}`, { ...init, signal: init?.signal ?? AbortSignal.timeout(10_000) })
  } catch {
    throw new Error('Cannot reach Ferrix. Check that the API is running, then refresh.')
  }
  if (!response.ok) {
    const body = await response.json().catch(() => null)
    throw new Error(body?.error ?? `Request failed (${response.status}). Check that the API is running.`)
  }
  if (response.status === 204) return undefined as T
  return response.json()
}

export async function load<T>(path: string): Promise<{ data: T; error: null } | { data: null; error: string }> {
  try { return { data: await request<T>(path), error: null } }
  catch (error) { return { data: null, error: error instanceof Error ? error.message : 'Unable to load data.' } }
}

export type HomeSummary = {
  projects: number; projects_new_7d: number
  services: number; applications: number; databases: number; compose: number; websites: number
  deployments_24h: number; succeeded_24h: number; failed_24h: number; in_progress_24h: number
  needs_attention: number
  attention: { id: string; name: string; project_id: string }[]
}
