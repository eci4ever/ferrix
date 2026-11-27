import type { ComponentProps, ReactNode } from 'react'
import { Button } from '@/components/ui/button'
import { Spinner } from '@/components/ui/spinner'

type LoadingButtonProps = Omit<ComponentProps<typeof Button>, 'asChild'> & {
  loading: boolean
  loadingText: ReactNode
}

export function LoadingButton({ loading, loadingText, disabled, children, ...props }: LoadingButtonProps) {
  return <Button {...props} disabled={disabled || loading} aria-busy={loading || undefined}>
    {loading ? <><Spinner aria-hidden="true" className="motion-reduce:animate-none" />{loadingText}</> : children}
  </Button>
}
