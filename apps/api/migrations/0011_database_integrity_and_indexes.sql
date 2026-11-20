-- Do not guess tenant ownership for legacy data. Resolve it before upgrading.
DO $$
BEGIN
    IF EXISTS (SELECT 1 FROM projects WHERE organization_id IS NULL) THEN
        RAISE EXCEPTION 'Projects without organization ownership must be assigned before this migration'
            USING ERRCODE = '23502';
    END IF;
END $$;
ALTER TABLE projects ALTER COLUMN organization_id SET NOT NULL;

-- The active deployment must belong to this exact service. Deferred checking
-- permits activation and cascading service deletion within one transaction.
ALTER TABLE deployments ADD CONSTRAINT deployments_application_id_id_key
    UNIQUE (application_id, id);
ALTER TABLE applications ADD CONSTRAINT applications_active_deployment_fkey
    FOREIGN KEY (id, active_deployment_id)
    REFERENCES deployments(application_id, id)
    DEFERRABLE INITIALLY DEFERRED;

CREATE INDEX auth_members_user_organization_idx ON auth_members(user_id, organization_id);
CREATE INDEX auth_audit_organization_created_idx ON auth_audit(organization_id, created_at DESC);

-- Match the bounded housekeeping queries, including either session deadline.
CREATE INDEX auth_sessions_expiry_idx ON auth_sessions((LEAST(expires_at, absolute_expires_at)), id);
CREATE INDEX auth_throttles_expiry_idx ON auth_throttles(reset_at, key);
CREATE INDEX auth_invitations_expiry_idx ON auth_invitations(expires_at, id);
CREATE INDEX auth_password_resets_expiry_idx ON auth_password_resets(expires_at, user_id);
CREATE INDEX auth_mfa_pending_expiry_idx ON auth_mfa(pending_expires_at, user_id) WHERE NOT enabled;
