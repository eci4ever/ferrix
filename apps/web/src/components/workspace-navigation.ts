import { Activity, Box, Folder, House, Settings, Users, UserRoundCog } from 'lucide-react'
import type { Identity } from '@/auth'
import type { NavigationItem } from '@/components/nav-main'

export function navigationGroups(identity: Pick<Identity, 'role' | 'system_role'>): { label: string; items: NavigationItem[] }[] {
  const groups: { label: string; items: NavigationItem[] }[] = [
    { label: 'Workspace', items: [{ title: 'Home', url: '/dashboard', icon: House }, { title: 'Projects', url: '/projects', icon: Folder }, { title: 'Monitoring', url: '/monitoring', icon: Activity }, { title: 'Docker', url: '/containers', icon: Box }] },
  ]
  if (identity.role === 'owner' || identity.role === 'admin') groups.push({ label: 'Organization', items: [{ title: 'Overview', url: '/organization/settings', icon: House }, { title: 'Members', url: '/organization/members', icon: Users }] })
  if (identity.system_role === 'admin') groups.push({ label: 'Platform', items: [{ title: 'Users', url: '/admin', icon: UserRoundCog }, { title: 'Organizations', url: '/platform/organizations', icon: Users }, { title: 'Settings', url: '/platform/settings', icon: Settings }] })
  return groups
}
