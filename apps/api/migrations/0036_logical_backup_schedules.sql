CREATE TABLE managed_backup_policies (
    managed_database_id uuid PRIMARY KEY REFERENCES managed_databases(id) ON DELETE CASCADE,
    enabled boolean NOT NULL DEFAULT false,
    cron text NOT NULL DEFAULT '0 2 * * *',
    timezone text NOT NULL DEFAULT 'UTC',
    keep_last integer NOT NULL DEFAULT 10 CHECK (keep_last BETWEEN 1 AND 365),
    configured_by uuid REFERENCES auth_users(id) ON DELETE SET NULL,
    next_run_at timestamptz,
    last_run_at timestamptz,
    last_message text,
    updated_at timestamptz NOT NULL DEFAULT now()
);
CREATE INDEX managed_backup_policies_due ON managed_backup_policies(next_run_at) WHERE enabled;
DROP INDEX database_jobs_schedule_unique;
CREATE UNIQUE INDEX database_jobs_service_schedule_unique ON database_jobs(application_id,scheduled_for)
    WHERE scheduled_for IS NOT NULL AND managed_database_id IS NULL;
CREATE UNIQUE INDEX database_jobs_logical_schedule_unique ON database_jobs(managed_database_id,scheduled_for)
    WHERE scheduled_for IS NOT NULL AND managed_database_id IS NOT NULL;
