import { Kbd, KbdGroup } from '@/components/ui/kbd'
import { useEffect, useState } from 'react'
import { useRouter } from '@tanstack/react-router'
import { Search } from 'lucide-react'
import type { Identity } from '@/auth'
import { navigationGroups } from '@/components/workspace-navigation'
import { Button } from '@/components/ui/button'
import { Command, CommandDialog, CommandEmpty, CommandGroup, CommandInput, CommandItem, CommandList } from '@/components/ui/command'
import { Tooltip, TooltipContent, TooltipTrigger } from '@/components/ui/tooltip'

export function WorkspaceCommand({ identity }: { identity: Identity }) {
  const [open, setOpen] = useState(false)
  const router = useRouter()
  useEffect(() => {
    const onKeyDown = (event: KeyboardEvent) => {
      if (event.key.toLowerCase() === 'k' && (event.metaKey || event.ctrlKey) && !event.altKey && !event.shiftKey && !event.repeat && !event.isComposing) {
        // Preserve terminal shortcuts while a terminal has focus.
        if (event.target instanceof Element && event.target.closest('.xterm')) return
        event.preventDefault()
        setOpen(value => !value)
      }
    }
    document.addEventListener('keydown', onKeyDown)
    return () => document.removeEventListener('keydown', onKeyDown)
  }, [])
  return <>
    <Tooltip><TooltipTrigger asChild><Button variant="outline" size="icon" aria-label="Search pages" aria-haspopup="dialog" aria-expanded={open} onClick={() => setOpen(true)}><Search aria-hidden="true" /></Button></TooltipTrigger><TooltipContent side="bottom">Search pages <KbdGroup><Kbd>⌘K</Kbd><span>/</span><Kbd>Ctrl+K</Kbd></KbdGroup></TooltipContent></Tooltip>
    <CommandDialog open={open} onOpenChange={setOpen} title="Search pages" description="Search available pages. Use arrow keys to choose and Enter to open." className="top-[10dvh] max-h-[80dvh] translate-y-0 sm:max-w-lg" showCloseButton>
      <Command key={identity.organization_id} className="min-h-0">
        <CommandInput aria-label="Search pages" placeholder="Search pages…" className="pr-9" />
        <CommandList className="max-h-[min(24rem,60dvh)]" aria-label="Pages"><CommandEmpty>No pages found.</CommandEmpty>
          {navigationGroups(identity).map(group => <CommandGroup key={group.label} heading={group.label}>{group.items.map(item => <CommandItem key={item.url} value={`${group.label} ${item.title}`} onSelect={() => { setOpen(false); void router.navigate({ to: item.url }) }}><item.icon aria-hidden="true" /><span>{item.title}</span></CommandItem>)}</CommandGroup>)}
        </CommandList>
        <div className="flex flex-wrap items-center gap-3 border-t px-3 py-2 text-xs text-muted-foreground"><span className="inline-flex items-center gap-1"><Kbd>↑</Kbd><Kbd>↓</Kbd> Navigate</span><span className="inline-flex items-center gap-1"><Kbd>Enter</Kbd> Open</span><span className="inline-flex items-center gap-1"><Kbd>Esc</Kbd> Close</span></div>
      </Command>
    </CommandDialog>
  </>
}
