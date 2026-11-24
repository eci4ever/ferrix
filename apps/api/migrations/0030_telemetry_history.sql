CREATE TABLE telemetry_settings (
 singleton BOOLEAN PRIMARY KEY DEFAULT true CHECK(singleton),
 sample_seconds BIGINT NOT NULL DEFAULT 30 CHECK(sample_seconds BETWEEN 10 AND 300),
 retention_days BIGINT NOT NULL DEFAULT 3 CHECK(retention_days BETWEEN 1 AND 30),
 log_retention_days BIGINT NOT NULL DEFAULT 3 CHECK(log_retention_days BETWEEN 1 AND 30),
 max_storage_mib BIGINT NOT NULL DEFAULT 128 CHECK(max_storage_mib BETWEEN 16 AND 1024),
 downsample_after_minutes BIGINT NOT NULL DEFAULT 360 CHECK(downsample_after_minutes BETWEEN 60 AND 10080),
 downsample_seconds BIGINT NOT NULL DEFAULT 300 CHECK(downsample_seconds BETWEEN 60 AND 3600),
 cpu_threshold DOUBLE PRECISION NOT NULL DEFAULT 90 CHECK(cpu_threshold BETWEEN 10 AND 100),
 memory_threshold DOUBLE PRECISION NOT NULL DEFAULT 90 CHECK(memory_threshold BETWEEN 10 AND 100),
 disk_threshold DOUBLE PRECISION NOT NULL DEFAULT 90 CHECK(disk_threshold BETWEEN 10 AND 100),
 sustained_seconds BIGINT NOT NULL DEFAULT 60 CHECK(sustained_seconds BETWEEN 30 AND 3600),
 restart_threshold BIGINT NOT NULL DEFAULT 3 CHECK(restart_threshold BETWEEN 1 AND 100),
 restart_window_minutes BIGINT NOT NULL DEFAULT 15 CHECK(restart_window_minutes BETWEEN 1 AND 60)
);
INSERT INTO telemetry_settings(singleton) VALUES(true);
CREATE TABLE telemetry_samples (
 id BIGSERIAL PRIMARY KEY,
 service_id UUID REFERENCES applications(id) ON DELETE CASCADE,
 deployment_id UUID REFERENCES deployments(id) ON DELETE SET NULL,
 container_id TEXT,
 image_id TEXT,
 collected_at TIMESTAMPTZ NOT NULL DEFAULT now(),
 data JSONB NOT NULL,
 error BOOLEAN NOT NULL DEFAULT false,
 marker BOOLEAN NOT NULL DEFAULT false
);
CREATE INDEX telemetry_samples_service_time ON telemetry_samples(service_id,collected_at DESC,id DESC);
CREATE INDEX telemetry_samples_time ON telemetry_samples(collected_at);
CREATE TABLE service_log_history (
 id BIGSERIAL PRIMARY KEY,
 service_id UUID NOT NULL REFERENCES applications(id) ON DELETE CASCADE,
 deployment_id UUID REFERENCES deployments(id) ON DELETE SET NULL,
 container_id TEXT NOT NULL,
 emitted_at TIMESTAMPTZ,
 collected_at TIMESTAMPTZ NOT NULL DEFAULT now(),
 message TEXT NOT NULL CHECK(octet_length(message)<=16384),
 fingerprint TEXT NOT NULL,
 truncated BOOLEAN NOT NULL DEFAULT false,
 UNIQUE(service_id,container_id,fingerprint)
);
CREATE INDEX service_log_history_service_time ON service_log_history(service_id,collected_at DESC,id DESC);
CREATE INDEX service_log_history_time ON service_log_history(collected_at);
CREATE TABLE telemetry_alerts (
 scope_id UUID NOT NULL,
 resource TEXT NOT NULL,
 active BOOLEAN NOT NULL DEFAULT false,
 high_since TIMESTAMPTZ,
 last_sample TIMESTAMPTZ,
 PRIMARY KEY(scope_id,resource)
);
