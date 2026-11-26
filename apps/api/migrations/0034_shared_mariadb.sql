-- Logical databases retain their data when the linked application is removed.
CREATE TABLE managed_databases (
    id uuid PRIMARY KEY,
    organization_id uuid NOT NULL REFERENCES auth_organizations(id),
    service_id uuid NOT NULL REFERENCES applications(id) ON DELETE RESTRICT,
    application_id uuid REFERENCES applications(id) ON DELETE SET NULL,
    database_name text NOT NULL,
    username text NOT NULL,
    credentials_cipher bytea NOT NULL,
    pending_credentials_cipher bytea,
    status text NOT NULL DEFAULT 'pending' CHECK(status IN ('pending','ready','failed','revoked','rotating','deleting')),
    max_connections integer NOT NULL DEFAULT 10 CHECK(max_connections BETWEEN 1 AND 50),
    message text NOT NULL DEFAULT 'Awaiting provisioning.',
    created_at timestamptz NOT NULL DEFAULT now(),
    updated_at timestamptz NOT NULL DEFAULT now(),
    UNIQUE(service_id,database_name),
    UNIQUE(service_id,username),
    UNIQUE(application_id)
);
CREATE INDEX managed_databases_organization ON managed_databases(organization_id);
ALTER TABLE database_jobs ADD COLUMN managed_database_id uuid REFERENCES managed_databases(id) ON DELETE RESTRICT;
ALTER TABLE database_jobs ADD COLUMN managed_database_name text;

CREATE FUNCTION ferrix_managed_database_link() RETURNS trigger LANGUAGE plpgsql AS $$
BEGIN
    IF NOT EXISTS (
        SELECT 1 FROM applications db JOIN environments e ON e.id=db.environment_id
        JOIN projects p ON p.id=e.project_id
        WHERE db.id=NEW.service_id AND db.service_kind='database' AND db.image LIKE 'mariadb:%'
        AND p.organization_id=NEW.organization_id
        AND (NEW.application_id IS NULL OR EXISTS (
            SELECT 1 FROM applications app WHERE app.id=NEW.application_id
            AND app.service_kind='application' AND app.environment_id=db.environment_id
        ))
    ) THEN RAISE EXCEPTION 'Managed database links require the same organization and environment'; END IF;
    RETURN NEW;
END $$;
CREATE TRIGGER managed_database_link BEFORE INSERT OR UPDATE OF service_id,application_id,organization_id ON managed_databases
FOR EACH ROW EXECUTE FUNCTION ferrix_managed_database_link();
