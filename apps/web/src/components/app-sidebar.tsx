import { Box, Folder, House, Shield, Users, UserRoundCog } from 'lucide-react'
import type { Identity, Organization } from '@/auth'
import { NavMain } from '@/components/nav-main'
import { NavUser } from '@/components/nav-user'
import { TeamSwitcher } from '@/components/team-switcher'
import { Sidebar, SidebarContent, SidebarFooter, SidebarHeader, SidebarRail, SidebarSeparator } from '@/components/ui/sidebar'

export function AppSidebar({ identity, organizations, theme, busy, onSwitch, onSignOut, onToggleTheme, version }: { identity: Identity; organizations: Organization[]; theme: 'dark' | 'light'; busy: boolean; onSwitch: (id: string) => Promise<void>; onSignOut: () => Promise<void>; onToggleTheme: () => void; version?: string }) {
  return <Sidebar collapsible="icon" variant="floating"><SidebarHeader><TeamSwitcher identity={identity} organizations={organizations} busy={busy} onSwitch={onSwitch} /></SidebarHeader><SidebarContent><nav aria-label="Main navigation"><NavMain label="Home" items={[{ title: 'Home', url: '/', icon: House }, { title: 'Projects', url: '/projects', icon: Folder }, ...(identity.system_role === 'admin' ? [{ title: 'Docker', url: '/containers' as const, icon: Box }] : [])]} /><NavMain label="Settings" items={[{ title: 'Security', url: '/security', icon: Shield }, { title: 'Organization', url: '/organization', icon: Users }, ...(identity.system_role === 'admin' ? [{ title: 'Users', url: '/admin' as const, icon: UserRoundCog }] : [])]} /></nav></SidebarContent><SidebarFooter><SidebarSeparator /><NavUser identity={identity} theme={theme} busy={busy} onSignOut={onSignOut} onToggleTheme={onToggleTheme} />{version ? <p className="text-center text-xs text-muted-foreground group-data-[collapsible=icon]:hidden">Version {version}</p> : null}</SidebarFooter><SidebarRail /></Sidebar>
}
