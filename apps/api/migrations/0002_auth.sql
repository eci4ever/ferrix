CREATE TABLE auth_users (
 id UUID PRIMARY KEY,
 name TEXT NOT NULL CHECK (char_length(name) BETWEEN 1 AND 80),
 email TEXT NOT NULL UNIQUE CHECK (email = lower(email)),
 password_hash TEXT NOT NULL,
 system_role TEXT NOT NULL DEFAULT 'user' CHECK (system_role IN ('admin','user')),
 banned BOOLEAN NOT NULL DEFAULT false,
 created_at TIMESTAMPTZ NOT NULL DEFAULT now()
);
CREATE TABLE auth_organizations (
 id UUID PRIMARY KEY,
 name TEXT NOT NULL CHECK (char_length(name) BETWEEN 1 AND 80),
 created_at TIMESTAMPTZ NOT NULL DEFAULT now()
);
CREATE TABLE auth_members (
 organization_id UUID NOT NULL REFERENCES auth_organizations(id) ON DELETE CASCADE,
 user_id UUID NOT NULL REFERENCES auth_users(id) ON DELETE CASCADE,
 role TEXT NOT NULL CHECK (role IN ('owner','admin','member')),
 PRIMARY KEY (organization_id,user_id)
);
CREATE TABLE auth_sessions (
 id UUID PRIMARY KEY,
 token_hash TEXT NOT NULL UNIQUE,
 user_id UUID NOT NULL REFERENCES auth_users(id) ON DELETE CASCADE,
 active_organization_id UUID NOT NULL REFERENCES auth_organizations(id) ON DELETE CASCADE,
 created_at TIMESTAMPTZ NOT NULL DEFAULT now(),
 updated_at TIMESTAMPTZ NOT NULL DEFAULT now(),
 authenticated_at TIMESTAMPTZ NOT NULL DEFAULT now(),
 expires_at TIMESTAMPTZ NOT NULL DEFAULT now() + interval '3 days',
 absolute_expires_at TIMESTAMPTZ NOT NULL DEFAULT now() + interval '30 days',
 user_agent TEXT NOT NULL DEFAULT '',
 ip_address TEXT NOT NULL DEFAULT ''
);
CREATE INDEX auth_sessions_user_idx ON auth_sessions(user_id);
CREATE TABLE auth_invitations (
 id UUID PRIMARY KEY,
 organization_id UUID NOT NULL REFERENCES auth_organizations(id) ON DELETE CASCADE,
 email TEXT NOT NULL,
 role TEXT NOT NULL CHECK (role IN ('admin','member')),
 token_hash TEXT NOT NULL UNIQUE,
 invited_by UUID NOT NULL REFERENCES auth_users(id),
 expires_at TIMESTAMPTZ NOT NULL DEFAULT now() + interval '2 days',
 accepted_at TIMESTAMPTZ,
 created_at TIMESTAMPTZ NOT NULL DEFAULT now()
);
CREATE TABLE auth_throttles (
 key TEXT PRIMARY KEY,
 attempts INTEGER NOT NULL,
 reset_at TIMESTAMPTZ NOT NULL
);
CREATE TABLE auth_audit (
 id UUID PRIMARY KEY,
 user_id UUID REFERENCES auth_users(id) ON DELETE SET NULL,
 organization_id UUID REFERENCES auth_organizations(id) ON DELETE SET NULL,
 action TEXT NOT NULL,
 created_at TIMESTAMPTZ NOT NULL DEFAULT now()
);
CREATE TABLE auth_mfa (
 user_id UUID PRIMARY KEY REFERENCES auth_users(id) ON DELETE CASCADE,
 secret BYTEA NOT NULL,
 enabled BOOLEAN NOT NULL DEFAULT false,
 last_step BIGINT NOT NULL DEFAULT -1,
 pending_expires_at TIMESTAMPTZ NOT NULL DEFAULT now() + interval '10 minutes'
);
CREATE TABLE auth_recovery_codes (
 user_id UUID NOT NULL REFERENCES auth_users(id) ON DELETE CASCADE,
 code_hash TEXT NOT NULL,
 PRIMARY KEY(user_id,code_hash)
);
ALTER TABLE projects ADD COLUMN organization_id UUID REFERENCES auth_organizations(id);
CREATE INDEX projects_organization_idx ON projects(organization_id, created_at DESC);
