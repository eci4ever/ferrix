CREATE TABLE database_jobs (
 id UUID PRIMARY KEY,
 application_id UUID NOT NULL REFERENCES applications(id) ON DELETE CASCADE,
 source_application_id UUID REFERENCES applications(id) ON DELETE SET NULL,
 source_backup_id UUID REFERENCES database_jobs(id) ON DELETE SET NULL,
 deployment_id UUID REFERENCES deployments(id) ON DELETE SET NULL,
 requested_by UUID REFERENCES auth_users(id) ON DELETE SET NULL,
 operation TEXT NOT NULL CHECK (operation IN ('backup','restore')),
 engine TEXT NOT NULL CHECK (engine IN ('postgres','mariadb')),
 image TEXT NOT NULL,
 image_id TEXT NOT NULL,
 config_cipher BYTEA NOT NULL,
 env_cipher BYTEA,
 status TEXT NOT NULL DEFAULT 'queued' CHECK (status IN ('queued','running','succeeded','failed')),
 message TEXT NOT NULL DEFAULT 'Waiting for worker.',
 size_bytes BIGINT,
 sha256 TEXT,
 created_at TIMESTAMPTZ NOT NULL DEFAULT now(),
 started_at TIMESTAMPTZ,
 finished_at TIMESTAMPTZ
);
CREATE UNIQUE INDEX database_jobs_one_pending ON database_jobs(application_id) WHERE status IN ('queued','running');
CREATE INDEX database_jobs_queue ON database_jobs(created_at,id) WHERE status='queued';
CREATE INDEX database_jobs_history ON database_jobs(application_id,created_at DESC);
CREATE INDEX database_jobs_source ON database_jobs(source_application_id,created_at DESC) WHERE source_application_id IS NOT NULL;
CREATE INDEX database_jobs_source_pending ON database_jobs(source_application_id) WHERE status IN ('queued','running') AND source_application_id IS NOT NULL;
