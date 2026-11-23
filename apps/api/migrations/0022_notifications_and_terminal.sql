CREATE TABLE notifications (
 id UUID PRIMARY KEY,
 organization_id UUID NOT NULL REFERENCES auth_organizations(id) ON DELETE CASCADE,
 project_id UUID NOT NULL REFERENCES projects(id) ON DELETE CASCADE,
 service_id UUID REFERENCES applications(id) ON DELETE SET NULL,
 kind TEXT NOT NULL,
 title TEXT NOT NULL,
 message TEXT NOT NULL,
 created_at TIMESTAMPTZ NOT NULL DEFAULT now(),
 expires_at TIMESTAMPTZ NOT NULL DEFAULT now()+interval '30 days'
);
CREATE INDEX notifications_org_created_idx ON notifications(organization_id,created_at DESC,id DESC);
CREATE INDEX notifications_project_idx ON notifications(project_id);
CREATE INDEX notifications_service_idx ON notifications(service_id) WHERE service_id IS NOT NULL;
CREATE INDEX notifications_expiry_idx ON notifications(expires_at);
CREATE TABLE notification_reads (
 notification_id UUID NOT NULL REFERENCES notifications(id) ON DELETE CASCADE,
 user_id UUID NOT NULL REFERENCES auth_users(id) ON DELETE CASCADE,
 read_at TIMESTAMPTZ NOT NULL DEFAULT now(),
 PRIMARY KEY(notification_id,user_id)
);
CREATE INDEX notification_reads_user_idx ON notification_reads(user_id);
-- Record only new transitions; do not replay historical deployments on migration.
CREATE FUNCTION ferrix_notify_outcome() RETURNS trigger LANGUAGE plpgsql AS $$
DECLARE service UUID; event_kind TEXT; event_title TEXT;
BEGIN
 IF NEW.status NOT IN ('succeeded','failed') OR NEW.status IS NOT DISTINCT FROM OLD.status THEN RETURN NEW; END IF;
 service := NEW.application_id;
 IF TG_TABLE_NAME='deployments' THEN
  -- Restore bootstrap is internal; the restore job emits the user-facing outcome.
  IF EXISTS(SELECT 1 FROM database_jobs WHERE deployment_id=NEW.id AND operation='restore') THEN RETURN NEW; END IF;
  event_kind := 'deployment_' || NEW.status;
  event_title := CASE NEW.status WHEN 'succeeded' THEN 'Deployment succeeded' ELSE 'Deployment failed' END;
 ELSE
  event_kind := NEW.operation || '_' || NEW.status;
  event_title := initcap(NEW.operation) || CASE NEW.status WHEN 'succeeded' THEN ' succeeded' ELSE ' failed' END;
 END IF;
 INSERT INTO notifications(id,organization_id,project_id,service_id,kind,title,message)
 SELECT gen_random_uuid(),p.organization_id,p.id,a.id,event_kind,event_title,a.name || ' · ' || p.name || ' / ' || e.name
 FROM applications a JOIN environments e ON e.id=a.environment_id JOIN projects p ON p.id=e.project_id WHERE a.id=service;
 RETURN NEW;
END $$;
CREATE TRIGGER deployment_notification AFTER UPDATE OF status ON deployments FOR EACH ROW EXECUTE FUNCTION ferrix_notify_outcome();
CREATE TRIGGER database_job_notification AFTER UPDATE OF status ON database_jobs FOR EACH ROW EXECUTE FUNCTION ferrix_notify_outcome();
CREATE TABLE service_resource_alerts (
 service_id UUID NOT NULL REFERENCES applications(id) ON DELETE CASCADE,
 container_id TEXT,
 resource TEXT NOT NULL CHECK(resource IN ('cpu','memory')),
 high_since TIMESTAMPTZ,
 last_sample TIMESTAMPTZ NOT NULL,
 notified BOOLEAN NOT NULL DEFAULT false,
 PRIMARY KEY(service_id,resource)
);
CREATE TABLE terminal_connections (
 id UUID PRIMARY KEY,
 user_id UUID NOT NULL REFERENCES auth_users(id) ON DELETE CASCADE,
 organization_id UUID NOT NULL REFERENCES auth_organizations(id) ON DELETE CASCADE,
 service_id UUID REFERENCES applications(id) ON DELETE SET NULL,
 container_id TEXT,
 exec_id TEXT,
 created_at TIMESTAMPTZ NOT NULL DEFAULT now(),
 expires_at TIMESTAMPTZ NOT NULL DEFAULT now()+interval '60 seconds',
 closed_at TIMESTAMPTZ
);
CREATE INDEX terminal_connections_user_active_idx ON terminal_connections(user_id,expires_at) WHERE closed_at IS NULL;
