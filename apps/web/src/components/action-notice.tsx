import { useEffect, useId } from 'react'
import { toast } from 'sonner'

export function ActionNotice({ message, onDismiss }: { message: string; onDismiss: () => void }) {
  const id = useId()
  useEffect(() => {
    if (!message) return
    toast.success(message, { id })
    onDismiss()
  }, [message, id, onDismiss])
  return null
}
