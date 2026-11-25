import type { ComponentProps, ReactElement, ReactNode } from 'react'
import { Dialog, DialogContent, DialogDescription, DialogHeader, DialogTitle, DialogTrigger } from '@/components/ui/dialog'
import { cn } from '@/lib/utils'

/** A focused form task. Keep drafts/errors mounted until dismissal or successful submission. */
export function FormDialog({ open, onOpenChange, title, description, busy = false, trigger, children, className, onCloseAutoFocus }: {
  open: boolean
  onOpenChange: (open: boolean) => void
  title: string
  description: string
  busy?: boolean
  trigger?: ReactElement
  children: ReactNode
  className?: string
  onCloseAutoFocus?: ComponentProps<typeof DialogContent>['onCloseAutoFocus']
}) {
  return <Dialog open={open} onOpenChange={next => { if (!busy) onOpenChange(next) }}>
    {trigger ? <DialogTrigger asChild>{trigger}</DialogTrigger> : null}
    <DialogContent className={cn('sm:max-w-lg', className)} showCloseButton={!busy}
      onEscapeKeyDown={event => { if (busy) event.preventDefault() }}
      onInteractOutside={event => { if (busy) event.preventDefault() }}
      onCloseAutoFocus={onCloseAutoFocus}>
      <DialogHeader><DialogTitle>{title}</DialogTitle><DialogDescription>{description}</DialogDescription></DialogHeader>
      {children}
    </DialogContent>
  </Dialog>
}
