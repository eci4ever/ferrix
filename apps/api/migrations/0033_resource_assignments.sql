ALTER TABLE auth_organizations ADD COLUMN restrict_member_resources boolean NOT NULL DEFAULT false;

CREATE TABLE project_assignments (
 organization_id uuid NOT NULL,
 project_id uuid NOT NULL,
 user_id uuid NOT NULL,
 permission text NOT NULL CHECK (permission IN ('viewer','developer')),
 PRIMARY KEY(project_id,user_id),
 FOREIGN KEY(project_id,organization_id) REFERENCES projects(id,organization_id) ON DELETE CASCADE,
 FOREIGN KEY(organization_id,user_id) REFERENCES auth_members(organization_id,user_id) ON DELETE CASCADE
);
CREATE INDEX project_assignments_member ON project_assignments(organization_id,user_id);
CREATE TABLE service_assignments (
 organization_id uuid NOT NULL,
 service_id uuid NOT NULL REFERENCES applications(id) ON DELETE CASCADE,
 user_id uuid NOT NULL,
 permission text NOT NULL CHECK (permission IN ('viewer','developer')),
 PRIMARY KEY(service_id,user_id),
 FOREIGN KEY(organization_id,user_id) REFERENCES auth_members(organization_id,user_id) ON DELETE CASCADE
);
CREATE INDEX service_assignments_member ON service_assignments(organization_id,user_id);
CREATE FUNCTION ferrix_validate_service_assignment() RETURNS trigger LANGUAGE plpgsql AS $$
BEGIN
 IF NOT EXISTS(SELECT 1 FROM applications a JOIN environments e ON e.id=a.environment_id JOIN projects p ON p.id=e.project_id WHERE a.id=NEW.service_id AND p.organization_id=NEW.organization_id) THEN
  RAISE EXCEPTION 'Assignment must belong to the service organization' USING ERRCODE='23514';
 END IF;
 RETURN NEW;
END $$;
CREATE TRIGGER service_assignment_tenant BEFORE INSERT OR UPDATE ON service_assignments FOR EACH ROW EXECUTE FUNCTION ferrix_validate_service_assignment();

-- Live membership checks also invalidate grants immediately after member removal.
CREATE FUNCTION ferrix_project_permission(actor uuid, org uuid, project uuid) RETURNS text LANGUAGE sql STABLE AS $$
 SELECT CASE WHEN m.role IN ('owner','admin') THEN 'manager'
   ELSE COALESCE(g.permission,CASE WHEN NOT o.restrict_member_resources THEN 'viewer' END) END
 FROM projects p JOIN auth_organizations o ON o.id=p.organization_id
 JOIN auth_members m ON m.organization_id=o.id AND m.user_id=actor
 JOIN auth_users u ON u.id=m.user_id AND NOT auth_user_banned(u.banned,u.ban_expires_at)
 LEFT JOIN project_assignments g ON g.project_id=p.id AND g.user_id=actor AND g.organization_id=o.id
 WHERE p.id=project AND o.id=org
$$;
CREATE FUNCTION ferrix_service_permission(actor uuid, org uuid, service uuid) RETURNS text LANGUAGE sql STABLE AS $$
 SELECT CASE WHEN m.role IN ('owner','admin') THEN 'manager'
   WHEN g.permission='developer' OR pg.permission='developer' THEN 'developer'
   WHEN g.permission='viewer' OR pg.permission='viewer' OR NOT o.restrict_member_resources THEN 'viewer' END
 FROM applications a JOIN environments e ON e.id=a.environment_id JOIN projects p ON p.id=e.project_id
 JOIN auth_organizations o ON o.id=p.organization_id
 JOIN auth_members m ON m.organization_id=o.id AND m.user_id=actor
 JOIN auth_users u ON u.id=m.user_id AND NOT auth_user_banned(u.banned,u.ban_expires_at)
 LEFT JOIN service_assignments g ON g.service_id=a.id AND g.user_id=actor AND g.organization_id=o.id
 LEFT JOIN project_assignments pg ON pg.project_id=p.id AND pg.user_id=actor AND pg.organization_id=o.id
 WHERE a.id=service AND o.id=org
$$;
CREATE FUNCTION ferrix_project_visible(actor uuid, org uuid, project uuid) RETURNS boolean LANGUAGE sql STABLE AS $$
 SELECT ferrix_project_permission(actor,org,project) IS NOT NULL OR EXISTS(
  SELECT 1 FROM applications a JOIN environments e ON e.id=a.environment_id
  WHERE e.project_id=project AND ferrix_service_permission(actor,org,a.id) IS NOT NULL)
$$;
CREATE FUNCTION ferrix_environment_visible(actor uuid, org uuid, environment uuid) RETURNS boolean LANGUAGE sql STABLE AS $$
 SELECT EXISTS(SELECT 1 FROM environments e WHERE e.id=environment AND (
  ferrix_project_permission(actor,org,e.project_id) IS NOT NULL OR EXISTS(
   SELECT 1 FROM applications a WHERE a.environment_id=e.id AND ferrix_service_permission(actor,org,a.id) IS NOT NULL)))
$$;
