CREATE TABLE application_git_sources (
 application_id UUID PRIMARY KEY REFERENCES applications(id) ON DELETE CASCADE,
 enabled BOOLEAN NOT NULL DEFAULT true,
 repository TEXT NOT NULL,
 branch TEXT NOT NULL,
 config_cipher BYTEA NOT NULL,
 revision UUID NOT NULL,
 auto_deploy BOOLEAN NOT NULL DEFAULT false,
 webhook_cipher BYTEA,
 configured_by UUID REFERENCES auth_users(id) ON DELETE SET NULL,
 pending_delivery UUID,
 pending_sha TEXT,
 pending_sender TEXT,
 last_received_at TIMESTAMPTZ,
 last_message TEXT,
 updated_at TIMESTAMPTZ NOT NULL DEFAULT now()
);
CREATE INDEX application_git_sources_pending_idx ON application_git_sources(application_id) WHERE pending_delivery IS NOT NULL;
CREATE TABLE github_webhook_deliveries (
 id UUID PRIMARY KEY DEFAULT gen_random_uuid(),
 payload_hash TEXT NOT NULL,
 application_id UUID NOT NULL REFERENCES applications(id) ON DELETE CASCADE,
 delivery_id UUID NOT NULL,
 received_at TIMESTAMPTZ NOT NULL DEFAULT now(),
 expires_at TIMESTAMPTZ NOT NULL DEFAULT now()+interval '30 days',
 UNIQUE(application_id,delivery_id),
 UNIQUE(application_id,payload_hash)
);
CREATE INDEX github_webhook_deliveries_expiry_idx ON github_webhook_deliveries(expires_at);
ALTER TABLE deployments ADD COLUMN source_cipher BYTEA;
ALTER TABLE deployments ADD COLUMN source_revision UUID;
ALTER TABLE deployments ADD COLUMN source_repository TEXT;
ALTER TABLE deployments ADD COLUMN source_branch TEXT;
ALTER TABLE deployments ADD COLUMN git_commit TEXT CHECK(git_commit IS NULL OR git_commit ~ '^[0-9a-f]{40}$');
ALTER TABLE deployments ADD COLUMN trigger_kind TEXT NOT NULL DEFAULT 'manual' CHECK(trigger_kind IN ('manual','webhook'));
ALTER TABLE deployments ADD COLUMN trigger_sender TEXT;
