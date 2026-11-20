-- Shared Hosting is retired. Preserve its metadata and external volumes for recovery.
UPDATE student_hosting_settings
SET enabled=false, registration_open=false, server_state='retired', server_error=NULL, updated_at=now();

-- Publicly registered Hosting accounts must not become Docker workspace accounts.
-- Preserve administrators and ordinary Docker/team accounts.
UPDATE auth_users u SET banned=true
WHERE u.system_role <> 'admin'
  AND EXISTS (SELECT 1 FROM student_hostings h WHERE h.user_id=u.id);
DELETE FROM auth_sessions s USING auth_users u
WHERE s.user_id=u.id AND u.system_role <> 'admin'
  AND EXISTS (SELECT 1 FROM student_hostings h WHERE h.user_id=u.id);
UPDATE student_hostings SET status='suspended'
WHERE status IN ('active','queued','provisioning');
