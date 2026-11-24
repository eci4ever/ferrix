CREATE TABLE application_source_uploads (
 id UUID PRIMARY KEY,
 application_id UUID NOT NULL REFERENCES applications(id) ON DELETE CASCADE,
 created_by UUID REFERENCES auth_users(id) ON DELETE SET NULL,
 archive BYTEA NOT NULL CHECK (octet_length(archive) <= 33554432),
 sha256 TEXT NOT NULL CHECK (sha256 ~ '^[a-f0-9]{64}$'),
 context TEXT NOT NULL,
 dockerfile TEXT NOT NULL,
 preview JSONB NOT NULL,
 created_at TIMESTAMPTZ NOT NULL DEFAULT now()
);
CREATE INDEX application_source_uploads_app_idx ON application_source_uploads(application_id,created_at DESC);
ALTER TABLE applications ADD COLUMN source_upload_id UUID REFERENCES application_source_uploads(id) ON DELETE SET NULL;
ALTER TABLE deployments ADD COLUMN source_upload_id UUID REFERENCES application_source_uploads(id) ON DELETE SET NULL;
ALTER TABLE deployments ADD COLUMN source_archive_sha256 TEXT CHECK(source_archive_sha256 IS NULL OR source_archive_sha256 ~ '^[a-f0-9]{64}$');
