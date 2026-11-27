import { Activity, Box, Folder, House, Settings, Users, UserRoundCog } from 'lucide-react'
import type { Identity, Organization } from '@/auth'
import { NavMain } from '@/components/nav-main'
import { ReleaseNotification } from '@/components/release-notification'
import { NavUser } from '@/components/nav-user'
import { TeamSwitcher } from '@/components/team-switcher'
import { Sidebar, SidebarContent, SidebarFooter, SidebarHeader, SidebarRail, SidebarSeparator } from '@/components/ui/sidebar'

export function AppSidebar({ identity, organizations, theme, busy, onSwitch, onSignOut, onToggleTheme, version }: { identity: Identity; organizations: Organization[]; theme: 'dark' | 'light'; busy: boolean; onSwitch: (id: string) => Promise<void>; onSignOut: () => Promise<void>; onToggleTheme: () => void; version?: string }) {
  const manager = identity.role === 'owner' || identity.role === 'admin'
  return <Sidebar collapsible="icon" variant="floating"><SidebarHeader><TeamSwitcher identity={identity} organizations={organizations} busy={busy} onSwitch={onSwitch} /></SidebarHeader><SidebarContent><nav aria-label="Main navigation">
    <NavMain label="Workspace" items={[{ title: 'Home', url: '/dashboard', icon: House }, { title: 'Projects', url: '/projects', icon: Folder }, { title: 'Monitoring', url: '/monitoring', icon: Activity }, { title: 'Docker', url: '/containers', icon: Box }]} />
    {manager ? <NavMain label="Organization" items={[{ title: 'Overview', url: '/organization/settings', icon: House }, { title: 'Members', url: '/organization/members', icon: Users }]} /> : null}
    {identity.system_role === 'admin' ? <NavMain label="Platform" items={[{ title: 'Users', url: '/admin', icon: UserRoundCog }, { title: 'Organizations', url: '/platform/organizations', icon: Users }, { title: 'Settings', url: '/platform/settings', icon: Settings }]} /> : null}
  </nav></SidebarContent><SidebarFooter>{identity.system_role === 'admin' ? <ReleaseNotification key={identity.user_id} /> : null}<SidebarSeparator /><NavUser identity={identity} theme={theme} busy={busy} onSignOut={onSignOut} onToggleTheme={onToggleTheme} />{version ? <p className="text-center text-xs text-muted-foreground group-data-[collapsible=icon]:hidden">Version {version}</p> : null}</SidebarFooter><SidebarRail /></Sidebar>
}
