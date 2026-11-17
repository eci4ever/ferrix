# Working in Ferrix

- Each macOS user owns a separate clone. Work only in the current user's clone; use GitHub to exchange changes. The standard checkout for each user is /Users/Shared/Workspace/TS/<user>/ferrix. The original /Users/Shared/Workspace/TS/ferrix is a legacy setup checkout; avoid concurrent edits there.
- Before edits, inspect git status and the current branch. Preserve existing changes. Use a task branch named <user>/<task>; main is the integration branch. Additional simultaneous agents belonging to one user need separate worktrees and distinct runtime ports.
- Agree on a task owner and scope before concurrent work. Coordinate changes to API contracts, migrations, dependencies and lockfiles with the other task owners. Record task claims in GitHub Issues; a local file is not a cross-clone lock.
- Submit work through a PR. nmfairus coordinates review and merging. Push to main, force-push, or destructive Git operations require explicit user authorization.
- For setup or running services, read docs/development.md. Use only the current user's Compose project, database and ports. Keep .env and credentials untracked. Preserve other users' containers, volumes and processes; avoid global Docker cleanup.
- New database changes use new migrations; preserve migrations already shared with the team.
- UI uses shadcn components and their semantic theme tokens, with Tailwind utilities for layout. Keep custom palettes and component styles out of global CSS.
- Validate relevant changes using Cargo formatting, Clippy and tests for Rust, and npm run build in apps/web for frontend changes. Run PostgreSQL integration tests against the current user's database. Report checks that could not run.
