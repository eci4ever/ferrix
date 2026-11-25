import { useSyncExternalStore } from 'react'
import type { CSSProperties } from 'react'
import { CircleCheck, Info, Loader2, OctagonX, TriangleAlert } from 'lucide-react'
import { Toaster as Sonner, type ToasterProps } from 'sonner'

function subscribeTheme(changed: () => void) {
  const observer = new MutationObserver(changed)
  observer.observe(document.documentElement, { attributes: true, attributeFilter: ['class'] })
  return () => observer.disconnect()
}

// shadcn Sonner wrapper, adapted to Ferrix's existing class-based theme.
export function Toaster(props: ToasterProps) {
  const dark = useSyncExternalStore(subscribeTheme, () => document.documentElement.classList.contains('dark'), () => false)
  return <Sonner theme={dark ? 'dark' : 'light'} className="toaster group" position="bottom-right" closeButton duration={6000}
    icons={{ success: <CircleCheck className="size-4 text-status-success" />, info: <Info className="size-4" />, warning: <TriangleAlert className="size-4 text-status-warning" />, error: <OctagonX className="size-4 text-destructive" />, loading: <Loader2 className="size-4 animate-spin motion-reduce:animate-none" /> }}
    style={{ '--normal-bg': 'var(--popover)', '--normal-text': 'var(--popover-foreground)', '--normal-border': 'var(--border)', '--border-radius': 'var(--radius)' } as CSSProperties}
    toastOptions={{ closeButtonAriaLabel: 'Dismiss notification', classNames: { toast: 'font-sans', title: 'whitespace-pre-wrap break-words', description: 'whitespace-pre-wrap break-words' } }}
    {...props} />
}
