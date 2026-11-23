CREATE TABLE backup_policies (
 application_id UUID PRIMARY KEY REFERENCES applications(id) ON DELETE CASCADE,
 enabled BOOLEAN NOT NULL DEFAULT false,
 cron TEXT NOT NULL DEFAULT '0 2 * * *',
 timezone TEXT NOT NULL DEFAULT 'UTC',
 keep_last INTEGER NOT NULL DEFAULT 10 CHECK(keep_last BETWEEN 1 AND 365),
 s3_cipher BYTEA,
 configured_by UUID REFERENCES auth_users(id) ON DELETE SET NULL,
 next_run_at TIMESTAMPTZ,
 last_run_at TIMESTAMPTZ,
 last_message TEXT,
 updated_at TIMESTAMPTZ NOT NULL DEFAULT now()
);
CREATE INDEX backup_policies_due_idx ON backup_policies(next_run_at) WHERE enabled;
ALTER TABLE database_jobs ADD COLUMN scheduled_for TIMESTAMPTZ;
ALTER TABLE database_jobs ADD COLUMN s3_cipher BYTEA;
ALTER TABLE database_jobs ADD COLUMN s3_key TEXT;
ALTER TABLE database_jobs ADD COLUMN pruning BOOLEAN NOT NULL DEFAULT false;
ALTER TABLE database_jobs ADD COLUMN pruned_at TIMESTAMPTZ;
ALTER TABLE database_jobs ADD COLUMN retention_error TEXT;
ALTER TABLE database_jobs ADD COLUMN prune_retry_at TIMESTAMPTZ;
CREATE UNIQUE INDEX database_jobs_schedule_unique ON database_jobs(application_id,scheduled_for) WHERE scheduled_for IS NOT NULL;
CREATE INDEX database_jobs_retention_idx ON database_jobs(application_id,created_at DESC) WHERE operation='backup' AND status='succeeded' AND pruned_at IS NULL;
CREATE INDEX database_jobs_pruning_idx ON database_jobs(prune_retry_at) WHERE pruning AND pruned_at IS NULL;
