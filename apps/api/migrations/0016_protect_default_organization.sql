ALTER TABLE auth_organizations ADD COLUMN is_default BOOLEAN NOT NULL DEFAULT false;
-- Prefer the installation audit event so renaming never changes which org is protected.
-- Older installations without that event retain their earliest surviving organization.
UPDATE auth_organizations SET is_default=true WHERE id=COALESCE(
    (SELECT organization_id FROM auth_audit WHERE action='installation.configured' AND organization_id IS NOT NULL ORDER BY created_at,id LIMIT 1),
    (SELECT id FROM auth_organizations ORDER BY created_at,id LIMIT 1)
);
CREATE UNIQUE INDEX auth_organizations_one_default_idx ON auth_organizations(is_default) WHERE is_default;
