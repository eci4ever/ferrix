ALTER TABLE auth_audit ADD COLUMN actor_id UUID, ADD COLUMN actor_name TEXT,
 ADD COLUMN organization_ref UUID, ADD COLUMN organization_name TEXT,
 ADD COLUMN target_id UUID, ADD COLUMN outcome TEXT NOT NULL DEFAULT 'succeeded'
 CHECK (outcome IN ('pending','succeeded','failed'));
UPDATE auth_audit a SET actor_id=a.user_id,organization_ref=a.organization_id,
 actor_name=(SELECT name FROM auth_users WHERE id=a.user_id),
 organization_name=(SELECT name FROM auth_organizations WHERE id=a.organization_id);
CREATE FUNCTION ferrix_audit_snapshot() RETURNS trigger LANGUAGE plpgsql AS $$
BEGIN
 NEW.actor_id := NEW.user_id;
 NEW.organization_ref := NEW.organization_id;
 NEW.actor_name := COALESCE((SELECT name FROM auth_users WHERE id=NEW.user_id),NEW.actor_name);
 NEW.organization_name := (SELECT name FROM auth_organizations WHERE id=NEW.organization_id);
 RETURN NEW;
END $$;
CREATE TRIGGER auth_audit_snapshot BEFORE INSERT ON auth_audit FOR EACH ROW EXECUTE FUNCTION ferrix_audit_snapshot();
CREATE INDEX auth_audit_activity_org ON auth_audit(organization_ref,created_at DESC,id DESC);
CREATE INDEX auth_audit_activity_actor ON auth_audit(actor_id,created_at DESC,id DESC);
-- Deployment lifecycle events share the deployment transaction, including worker/webhook changes.
CREATE FUNCTION ferrix_deployment_audit() RETURNS trigger LANGUAGE plpgsql AS $$
DECLARE org UUID;
BEGIN
 IF TG_OP='UPDATE' AND NEW.status IS NOT DISTINCT FROM OLD.status THEN RETURN NEW; END IF;
 IF NEW.status NOT IN ('queued','succeeded','failed') THEN RETURN NEW; END IF;
 SELECT p.organization_id INTO org FROM applications a JOIN environments e ON e.id=a.environment_id
 JOIN projects p ON p.id=e.project_id WHERE a.id=NEW.application_id;
 IF org IS NOT NULL THEN
  INSERT INTO auth_audit(id,user_id,organization_id,action,target_id,outcome,actor_name)
  VALUES(gen_random_uuid(),NEW.requested_by,org,'deployment.'||NEW.status,NEW.application_id,
   CASE WHEN NEW.status='failed' THEN 'failed' ELSE 'succeeded' END,NEW.requested_by_name);
 END IF;
 RETURN NEW;
END $$;
CREATE TRIGGER deployments_activity AFTER INSERT OR UPDATE OF status ON deployments
 FOR EACH ROW EXECUTE FUNCTION ferrix_deployment_audit();
