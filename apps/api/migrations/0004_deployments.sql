CREATE TABLE environments (
 id UUID PRIMARY KEY,
 project_id UUID NOT NULL REFERENCES projects(id) ON DELETE CASCADE,
 name TEXT NOT NULL CHECK (char_length(name) BETWEEN 1 AND 80),
 created_at TIMESTAMPTZ NOT NULL DEFAULT now(),
 UNIQUE(project_id, name)
);
-- Existing projects receive the same initial environment as new projects.
INSERT INTO environments(id, project_id, name)
SELECT gen_random_uuid(), id, 'production' FROM projects;
CREATE TABLE applications (
 id UUID PRIMARY KEY,
 environment_id UUID NOT NULL REFERENCES environments(id) ON DELETE CASCADE,
 name TEXT NOT NULL CHECK (char_length(name) BETWEEN 1 AND 80),
 image TEXT NOT NULL CHECK (char_length(image) BETWEEN 1 AND 255),
 container_port INTEGER NOT NULL CHECK (container_port BETWEEN 1 AND 65535),
 env_cipher BYTEA,
 active_container_id TEXT,
 active_deployment_id UUID,
 host_port INTEGER CHECK (host_port BETWEEN 1 AND 65535),
 created_at TIMESTAMPTZ NOT NULL DEFAULT now(),
 updated_at TIMESTAMPTZ NOT NULL DEFAULT now(),
 UNIQUE(environment_id, name)
);
CREATE TABLE deployments (
 id UUID PRIMARY KEY,
 application_id UUID NOT NULL REFERENCES applications(id) ON DELETE CASCADE,
 requested_by UUID REFERENCES auth_users(id),
 image TEXT NOT NULL,
 container_port INTEGER NOT NULL,
 env_cipher BYTEA,
 status TEXT NOT NULL DEFAULT 'queued' CHECK (status IN ('queued','running','succeeded','failed')),
 logs TEXT NOT NULL DEFAULT '',
 container_id TEXT,
 image_id TEXT,
 created_at TIMESTAMPTZ NOT NULL DEFAULT now(),
 started_at TIMESTAMPTZ,
 finished_at TIMESTAMPTZ
);
CREATE UNIQUE INDEX deployments_one_active ON deployments(application_id) WHERE status IN ('queued','running');
CREATE INDEX deployments_queue ON deployments(created_at,id) WHERE status='queued';
CREATE INDEX deployments_history ON deployments(application_id,created_at DESC);
