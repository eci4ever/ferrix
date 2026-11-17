import { Link } from '@tanstack/react-router'
import { Layers3 } from 'lucide-react'
import { Button } from '@/components/ui/button'

export function LandingPage() {
  return <div className="min-h-svh bg-background text-foreground">
    <header className="border-b"><div className="mx-auto flex max-w-5xl items-center justify-between gap-4 px-6 py-4">
      <Link to="/" className="flex items-center gap-2 font-semibold" aria-label="Ferrix home"><Layers3 className="size-5" aria-hidden="true" />Ferrix</Link>
      <nav aria-label="Account navigation" className="flex gap-2"><Button asChild variant="ghost"><Link to="/login">Login</Link></Button><Button asChild><Link to="/signup">Sign up</Link></Button></nav>
    </div></header>
    <main className="mx-auto flex max-w-5xl flex-col items-start gap-6 px-6 py-24">
      <h1 className="max-w-xl text-3xl font-semibold tracking-tight">Your apps. Your infrastructure.</h1>
      <p className="max-w-lg text-muted-foreground">A self-hosted workspace to organize your projects and manage your applications.</p>
      <div className="flex flex-wrap gap-2"><Button asChild><Link to="/signup">Get started</Link></Button><Button asChild variant="outline"><Link to="/login">Login</Link></Button></div>
    </main>
  </div>
}
