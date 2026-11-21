ALTER TABLE auth_users ADD COLUMN ban_reason TEXT CHECK (char_length(ban_reason)<=500);
ALTER TABLE auth_users ADD COLUMN ban_expires_at TIMESTAMPTZ;
ALTER TABLE auth_organizations ADD COLUMN logo_url TEXT CHECK (char_length(logo_url)<=2048);
-- Use effective ban state everywhere; an elapsed temporary ban never requires a cleanup job to permit login.
CREATE FUNCTION auth_user_banned(is_banned BOOLEAN, deadline TIMESTAMPTZ)
RETURNS BOOLEAN LANGUAGE SQL STABLE AS $$ SELECT is_banned AND (deadline IS NULL OR deadline>now()) $$;
CREATE INDEX auth_users_created_idx ON auth_users(created_at DESC,id);
CREATE INDEX auth_users_role_created_idx ON auth_users(system_role,created_at DESC,id);
