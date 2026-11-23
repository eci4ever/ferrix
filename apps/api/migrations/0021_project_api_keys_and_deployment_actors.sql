ALTER TABLE projects ADD CONSTRAINT projects_id_organization_unique UNIQUE(id,organization_id);
ALTER TABLE auth_api_keys ADD COLUMN project_id UUID;
ALTER TABLE auth_api_keys ADD CONSTRAINT auth_api_keys_project_organization_fk
 FOREIGN KEY(project_id,organization_id) REFERENCES projects(id,organization_id) ON DELETE CASCADE;
CREATE INDEX auth_api_keys_project_idx ON auth_api_keys(project_id) WHERE project_id IS NOT NULL;
ALTER TABLE deployments ADD COLUMN requested_by_name TEXT;
ALTER TABLE deployments ADD COLUMN requested_by_key_id UUID REFERENCES auth_api_keys(id) ON DELETE SET NULL;
ALTER TABLE deployments ADD COLUMN requested_by_key_name TEXT;
UPDATE deployments d SET requested_by_name=u.name FROM auth_users u WHERE u.id=d.requested_by;
CREATE INDEX deployments_requested_key_idx ON deployments(requested_by_key_id) WHERE requested_by_key_id IS NOT NULL;
