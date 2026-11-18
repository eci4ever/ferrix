ALTER TABLE applications ADD COLUMN service_kind TEXT NOT NULL DEFAULT 'application' CHECK (service_kind IN ('application','database','compose'));
ALTER TABLE applications ADD COLUMN service_cipher BYTEA;
ALTER TABLE deployments ADD COLUMN service_kind TEXT NOT NULL DEFAULT 'application' CHECK (service_kind IN ('application','database','compose'));
ALTER TABLE deployments ADD COLUMN service_cipher BYTEA;
ALTER TABLE deployments ADD COLUMN restore_container_id TEXT;
