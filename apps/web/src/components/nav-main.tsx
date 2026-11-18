import { Link, useRouterState } from '@tanstack/react-router'
import type { LucideIcon } from 'lucide-react'
import { SidebarGroup, SidebarGroupLabel, SidebarMenu, SidebarMenuButton, SidebarMenuItem, useSidebar } from '@/components/ui/sidebar'

export type NavigationItem = { title: string; url: '/dashboard' | '/projects' | '/monitoring' | '/containers' | '/security' | '/organization' | '/admin'; icon: LucideIcon }
export function NavMain({ label, items }: { label: string; items: NavigationItem[] }) {
  const pathname = useRouterState({ select: state => state.location.pathname })
  const { setOpenMobile } = useSidebar()
  return <SidebarGroup><SidebarGroupLabel>{label}</SidebarGroupLabel><SidebarMenu>{items.map(item => <SidebarMenuItem key={item.url}><SidebarMenuButton asChild tooltip={item.title} isActive={pathname === item.url || pathname.startsWith(`${item.url}/`)}><Link to={item.url} onClick={() => setOpenMobile(false)}><item.icon /><span>{item.title}</span></Link></SidebarMenuButton></SidebarMenuItem>)}</SidebarMenu></SidebarGroup>
}
