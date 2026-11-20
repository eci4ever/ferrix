import { CheckCircle2, X } from 'lucide-react'
import { Alert, AlertTitle, AlertAction } from '@/components/ui/alert'
import { Button } from '@/components/ui/button'

export function ActionNotice({ message, onDismiss }: { message: string; onDismiss: () => void }) {
  if (!message) return null
  return <Alert role="status" aria-live="polite"><CheckCircle2 aria-hidden="true" className="text-status-success" /><AlertTitle className="break-words">{message}</AlertTitle><AlertAction><Button type="button" variant="ghost" size="icon" className="size-7" aria-label="Dismiss notification" onClick={onDismiss}><X /></Button></AlertAction></Alert>
}
