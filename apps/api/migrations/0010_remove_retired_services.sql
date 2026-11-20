-- Retire legacy service records without touching Application/Database services.
DELETE FROM applications WHERE service_kind='compose';
ALTER TABLE applications DROP CONSTRAINT applications_service_kind_check;
ALTER TABLE applications ADD CHECK (service_kind IN ('application','database'));
ALTER TABLE deployments DROP CONSTRAINT deployments_service_kind_check;
ALTER TABLE deployments ADD CHECK (service_kind IN ('application','database'));
DROP TABLE static_files, static_websites, static_releases;

CREATE TEMP TABLE ferrix_retired_accounts ON COMMIT DROP AS
SELECT u.id, h.organization_id FROM auth_users u
JOIN student_hostings h ON h.user_id=u.id WHERE u.system_role <> 'admin';
DROP TABLE student_hostings, student_hosting_settings;
DROP SEQUENCE student_hosting_unix_uid;
UPDATE deployments SET requested_by=NULL
WHERE requested_by IN (SELECT id FROM ferrix_retired_accounts);
DELETE FROM auth_invitations WHERE invited_by IN (SELECT id FROM ferrix_retired_accounts);
DELETE FROM auth_audit WHERE user_id IN (SELECT id FROM ferrix_retired_accounts);
DELETE FROM auth_users WHERE id IN (SELECT id FROM ferrix_retired_accounts);
DELETE FROM auth_organizations o
WHERE o.id IN (SELECT organization_id FROM ferrix_retired_accounts)
AND NOT EXISTS (SELECT 1 FROM auth_members m WHERE m.organization_id=o.id)
AND NOT EXISTS (SELECT 1 FROM projects p WHERE p.organization_id=o.id);
