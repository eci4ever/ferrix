import { useEffect, useState } from 'react'
import { request } from '@/lib/api'
import { Link } from '@tanstack/react-router'
import { ChevronsUpDown, LogOut, Moon, UserRound, Sun } from 'lucide-react'
import type { Identity } from '@/auth'
import { Avatar, AvatarFallback, AvatarImage } from '@/components/ui/avatar'
import { DropdownMenu, DropdownMenuContent, DropdownMenuGroup, DropdownMenuItem, DropdownMenuLabel, DropdownMenuSeparator, DropdownMenuTrigger } from '@/components/ui/dropdown-menu'
import { SidebarMenu, SidebarMenuButton, SidebarMenuItem, useSidebar } from '@/components/ui/sidebar'

export function NavUser({ identity, theme, busy, onSignOut, onToggleTheme }: { identity: Identity; theme: 'dark' | 'light'; busy: boolean; onSignOut: () => Promise<void>; onToggleTheme: () => void }) {
  const { isMobile, setOpenMobile } = useSidebar()
  const [avatarUrl, setAvatarUrl] = useState<string | null>(null)
  const [avatar, setAvatar] = useState('initials')
  useEffect(() => {
    const controller = new AbortController()
    request<{ avatar: string; avatar_url?: string | null }>('/auth/profile', { signal: controller.signal }).then(profile => { if (!controller.signal.aborted) { setAvatar(profile.avatar); setAvatarUrl(profile.avatar_url ?? null) } }).catch(() => {})
    return () => controller.abort()
  }, [identity])
  const initials = identity.name.trim().split(/\s+/).slice(0, 2).map(part => part[0]).join('').toUpperCase() || 'F'
  return <SidebarMenu><SidebarMenuItem><DropdownMenu><DropdownMenuTrigger asChild><SidebarMenuButton size="lg" aria-label="Account menu"><Avatar>{avatarUrl && <AvatarImage src={avatarUrl} alt="" referrerPolicy="no-referrer" />}<AvatarFallback>{avatar === 'initials' ? initials : avatar}</AvatarFallback></Avatar><div className="grid flex-1 text-left text-sm leading-tight"><span className="truncate font-semibold">{identity.name}</span><span className="truncate text-xs">{identity.email}</span></div><ChevronsUpDown className="ml-auto" /></SidebarMenuButton></DropdownMenuTrigger><DropdownMenuContent side={isMobile ? 'bottom' : 'right'} align="end" sideOffset={4}><DropdownMenuLabel><span className="block">{identity.name}</span><span className="block truncate text-xs font-normal text-muted-foreground">{identity.email}</span></DropdownMenuLabel><DropdownMenuSeparator /><DropdownMenuGroup><DropdownMenuItem asChild><Link to="/profile" onClick={() => setOpenMobile(false)}><UserRound />Profile</Link></DropdownMenuItem><DropdownMenuItem onSelect={onToggleTheme}>{theme === 'dark' ? <Sun /> : <Moon />}{theme === 'dark' ? 'Light theme' : 'Dark theme'}</DropdownMenuItem></DropdownMenuGroup><DropdownMenuSeparator /><DropdownMenuGroup><DropdownMenuItem disabled={busy} onSelect={() => { void onSignOut() }}><LogOut />{busy ? 'Signing out…' : 'Sign out'}</DropdownMenuItem></DropdownMenuGroup></DropdownMenuContent></DropdownMenu></SidebarMenuItem></SidebarMenu>
}
