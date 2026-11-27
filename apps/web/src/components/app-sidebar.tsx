import { navigationGroups } from '@/components/workspace-navigation'
import type { Identity, Organization } from '@/auth'
import { NavMain } from '@/components/nav-main'
import { ReleaseNotification } from '@/components/release-notification'
import { NavUser } from '@/components/nav-user'
import { TeamSwitcher } from '@/components/team-switcher'
import { Sidebar, SidebarContent, SidebarFooter, SidebarHeader, SidebarRail, SidebarSeparator } from '@/components/ui/sidebar'

export function AppSidebar({ identity, organizations, theme, busy, onSwitch, onSignOut, onToggleTheme, version }: { identity: Identity; organizations: Organization[]; theme: 'dark' | 'light'; busy: boolean; onSwitch: (id: string) => Promise<void>; onSignOut: () => Promise<void>; onToggleTheme: () => void; version?: string }) {
  return <Sidebar collapsible="icon" variant="floating"><SidebarHeader><TeamSwitcher identity={identity} organizations={organizations} busy={busy} onSwitch={onSwitch} /></SidebarHeader><SidebarContent><nav aria-label="Main navigation">
    {navigationGroups(identity).map(group => <NavMain key={group.label} label={group.label} items={group.items} />)}
  </nav></SidebarContent><SidebarFooter>{identity.system_role === 'admin' ? <ReleaseNotification key={identity.user_id} /> : null}<SidebarSeparator /><NavUser identity={identity} theme={theme} busy={busy} onSignOut={onSignOut} onToggleTheme={onToggleTheme} />{version ? <p className="text-center text-xs text-muted-foreground group-data-[collapsible=icon]:hidden">Version {version}</p> : null}</SidebarFooter><SidebarRail /></Sidebar>
}
