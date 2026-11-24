import { useEffect, useState } from 'react'
import { ArrowUpCircle, ExternalLink } from 'lucide-react'
import { SelfUpdateControls, useSelfUpdate } from '@/components/self-update-controls'
import { request } from '@/lib/api'
import { Button } from '@/components/ui/button'
import { Dialog, DialogContent, DialogDescription, DialogHeader, DialogTitle, DialogTrigger } from '@/components/ui/dialog'
import { SidebarMenu, SidebarMenuItem, SidebarMenuButton } from '@/components/ui/sidebar'

type ReleaseStatus = {
  current_version: string
  latest_version: string | null
  update_available: boolean
  release_notes_url: string | null
  upgrade_guide_url: string
  checked_at: string | null
  check_failed: boolean
}

// The browser only reads the server cache; GitHub is checked by the backend every six hours.
export function ReleaseNotification() {
  const update = useSelfUpdate()
  const [release, setRelease] = useState<ReleaseStatus | null>(null)
  useEffect(() => {
    const controller = new AbortController()
    const refresh = async () => {
      if (document.visibilityState === 'hidden') return
      try {
        const next = await request<ReleaseStatus>('/releases/latest', { signal: AbortSignal.any([controller.signal, AbortSignal.timeout(10_000)]) })
        if (!controller.signal.aborted) setRelease(next)
      } catch {
        if (!controller.signal.aborted) setRelease(current => current ? { ...current, check_failed: true } : null)
      }
    }
    void refresh()
    const timer = window.setInterval(() => void refresh(), 15 * 60_000)
    document.addEventListener('visibilitychange', refresh)
    return () => {
      controller.abort()
      window.clearInterval(timer)
      document.removeEventListener('visibilitychange', refresh)
    }
  }, [])

  if ((!release?.update_available || !release.latest_version || !release.release_notes_url) && !update.seen && !update.active) return null
  const target = update.active ? update.status?.target_version ?? release?.latest_version : release?.latest_version ?? update.status?.target_version
  const completed = update.status?.stage === 'succeeded' && update.status.target_version === release?.latest_version
  const available = Boolean(release?.update_available && !release.check_failed && !completed)
  const notice = update.active ? 'Ferrix update in progress' : completed ? 'Ferrix update completed' : `Ferrix v${target} available`

  return <Dialog>
    <SidebarMenu><SidebarMenuItem>
      <DialogTrigger asChild>
        <SidebarMenuButton aria-label={notice} tooltip={notice}>
          <ArrowUpCircle aria-hidden="true" />
          <span className="truncate">{update.active ? 'Updating Ferrix…' : completed ? 'Ferrix updated' : `Update available · v${target}`}</span>
        </SidebarMenuButton>
      </DialogTrigger>
    </SidebarMenuItem></SidebarMenu>
    <DialogContent className="max-h-[90dvh] overflow-y-auto">
      <DialogHeader>
        <DialogTitle>{update.seen || update.active ? 'Ferrix update' : `Ferrix v${target} is available`}</DialogTitle>
        <DialogDescription>You are running v{release?.current_version ?? '…'}. Review the release notes before upgrading.</DialogDescription>
      </DialogHeader>
      {!update.status?.supported ? <ol className="list-decimal pl-5 text-sm flex flex-col gap-2">
        <li>Back up PostgreSQL, your .env file and service data. Preserve FERRIX_AUTH_KEY.</li>
        <li>Check migration and compatibility notes for the new release.</li>
        <li>Follow the upgrade guide to update the image and restart Ferrix.</li>
      </ol> : null}
      <p className="text-sm text-muted-foreground">Updates require administrator confirmation.</p>
      {release?.checked_at ? <p className="text-xs text-muted-foreground">Last checked: {new Date(release.checked_at).toLocaleString()}{release.check_failed ? '. The latest check failed; showing the last confirmed release.' : '. Checked automatically every 6 hours.'}</p> : null}
      <SelfUpdateControls update={update} version={target ?? null} available={available} />
      <div className="flex flex-wrap gap-2">
        <Button asChild><a href={release?.release_notes_url ?? 'https://github.com/eci4ever/ferrix/releases/latest'} target="_blank" rel="noopener noreferrer">Release notes<ExternalLink data-icon="inline-end" /></a></Button>
        <Button variant="outline" asChild><a href={release?.upgrade_guide_url ?? 'https://github.com/eci4ever/ferrix/blob/main/docs/production.md#operate-and-update'} target="_blank" rel="noopener noreferrer">Upgrade guide<ExternalLink data-icon="inline-end" /></a></Button>
      </div>
    </DialogContent>
  </Dialog>
}
