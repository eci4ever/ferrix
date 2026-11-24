CREATE TABLE project_bundle_jobs (
 id UUID PRIMARY KEY,
 organization_id UUID NOT NULL REFERENCES auth_organizations(id) ON DELETE CASCADE,
 project_id UUID REFERENCES projects(id) ON DELETE CASCADE,
 target_project_id UUID REFERENCES projects(id) ON DELETE CASCADE,
 requested_by UUID REFERENCES auth_users(id) ON DELETE SET NULL,
 operation TEXT NOT NULL CHECK(operation IN ('backup','restore')),
 status TEXT NOT NULL DEFAULT 'queued' CHECK(status IN ('preview','queued','running','resuming','succeeded','failed','restored','cutover_queued','cutting_over','active')),
 message TEXT NOT NULL DEFAULT 'Queued.',
 project_name TEXT NOT NULL,
 key_cipher BYTEA,
 manifest_cipher BYTEA,
 artifact BYTEA CHECK(octet_length(artifact)<=67110000),
 sha256 TEXT,
 mapping JSONB NOT NULL DEFAULT '[]',
 created_at TIMESTAMPTZ NOT NULL DEFAULT now(),
 finished_at TIMESTAMPTZ
);
CREATE INDEX project_bundle_jobs_org_idx ON project_bundle_jobs(organization_id,created_at DESC);
CREATE TABLE project_maintenance (
 project_id UUID PRIMARY KEY REFERENCES projects(id) ON DELETE CASCADE,
 job_id UUID NOT NULL REFERENCES project_bundle_jobs(id) ON DELETE CASCADE
);
CREATE TABLE project_bundle_members (
 job_id UUID NOT NULL REFERENCES project_bundle_jobs(id) ON DELETE CASCADE,
 source_service_id UUID NOT NULL,
 target_service_id UUID REFERENCES applications(id) ON DELETE SET NULL,
 container_id TEXT,
 restart_policy JSONB,
 was_running BOOLEAN NOT NULL DEFAULT false,
 resume_needed BOOLEAN NOT NULL DEFAULT false,
 stop_confirmed BOOLEAN NOT NULL DEFAULT false,
 backup_id UUID REFERENCES database_jobs(id) ON DELETE SET NULL,
 PRIMARY KEY(job_id,source_service_id)
);
ALTER TABLE applications ADD COLUMN traffic_enabled BOOLEAN NOT NULL DEFAULT true;
ALTER TABLE applications ADD COLUMN cutover_pending BOOLEAN NOT NULL DEFAULT false;
ALTER TABLE database_jobs ADD COLUMN project_bundle_id UUID REFERENCES project_bundle_jobs(id) ON DELETE SET NULL;
ALTER TABLE deployments ADD COLUMN project_bundle_id UUID REFERENCES project_bundle_jobs(id) ON DELETE SET NULL;
CREATE FUNCTION ferrix_project_maintenance_guard() RETURNS trigger LANGUAGE plpgsql AS $$
DECLARE project UUID; app UUID; locked UUID;
BEGIN
 IF TG_TABLE_NAME='projects' THEN project:=COALESCE(NEW.id,OLD.id);
 ELSIF TG_TABLE_NAME='environments' THEN project:=COALESCE(NEW.project_id,OLD.project_id);
 ELSIF TG_TABLE_NAME='applications' THEN SELECT project_id INTO project FROM environments WHERE id=COALESCE(NEW.environment_id,OLD.environment_id);
 ELSE app:=COALESCE(NEW.application_id,OLD.application_id); SELECT e.project_id INTO project FROM applications a JOIN environments e ON e.id=a.environment_id WHERE a.id=app;
 END IF;
 IF project IS NOT NULL THEN
  -- Fail immediately for an established maintenance window before waiting on a
  -- project row; lifecycle writers already hold their service row lock.
  SELECT job_id INTO locked FROM project_maintenance WHERE project_id=project;
  IF locked IS NOT NULL AND locked::text IS DISTINCT FROM current_setting('ferrix.bundle_context',true) THEN
   RAISE EXCEPTION 'Project is in coordinated backup/restore maintenance.' USING ERRCODE='23514',CONSTRAINT='ferrix_project_maintenance';
  END IF;
  PERFORM id FROM projects WHERE id=project FOR UPDATE;
  SELECT job_id INTO locked FROM project_maintenance WHERE project_id=project;
  IF locked IS NOT NULL AND locked::text IS DISTINCT FROM current_setting('ferrix.bundle_context',true) THEN
   RAISE EXCEPTION 'Project is in coordinated backup/restore maintenance.' USING ERRCODE='23514',CONSTRAINT='ferrix_project_maintenance';
  END IF;
 END IF;
 IF TG_OP='DELETE' THEN RETURN OLD; END IF; RETURN NEW;
END $$;
CREATE TRIGGER ferrix_project_guard BEFORE UPDATE OF name,description OR DELETE ON projects FOR EACH ROW EXECUTE FUNCTION ferrix_project_maintenance_guard();
CREATE TRIGGER ferrix_environment_guard BEFORE INSERT OR UPDATE OR DELETE ON environments FOR EACH ROW EXECUTE FUNCTION ferrix_project_maintenance_guard();
CREATE TRIGGER ferrix_application_guard BEFORE INSERT OR UPDATE OF name,image,container_port,env_cipher,service_cipher,worker_mode,environment_id,source_upload_id OR DELETE ON applications FOR EACH ROW EXECUTE FUNCTION ferrix_project_maintenance_guard();
CREATE TRIGGER ferrix_deployment_guard BEFORE INSERT ON deployments FOR EACH ROW EXECUTE FUNCTION ferrix_project_maintenance_guard();
CREATE TRIGGER ferrix_database_job_guard BEFORE INSERT ON database_jobs FOR EACH ROW EXECUTE FUNCTION ferrix_project_maintenance_guard();
CREATE TRIGGER ferrix_domain_guard BEFORE INSERT OR UPDATE OR DELETE ON application_domains FOR EACH ROW EXECUTE FUNCTION ferrix_project_maintenance_guard();
