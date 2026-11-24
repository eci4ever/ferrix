ALTER TABLE applications ADD COLUMN worker_mode boolean NOT NULL DEFAULT false;
ALTER TABLE deployments ADD COLUMN worker_mode boolean NOT NULL DEFAULT false;
ALTER TABLE deployments ADD COLUMN restore_was_running boolean NOT NULL DEFAULT true;
ALTER TABLE applications ADD CONSTRAINT application_worker_kind CHECK (NOT worker_mode OR service_kind='application');
ALTER TABLE deployments ADD CONSTRAINT deployment_worker_kind CHECK (NOT worker_mode OR service_kind='application');
