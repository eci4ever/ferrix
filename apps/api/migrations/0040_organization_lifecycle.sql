ALTER TABLE auth_users ADD COLUMN last_sign_in_at TIMESTAMPTZ;
ALTER TABLE auth_members ADD COLUMN joined_at TIMESTAMPTZ;
ALTER TABLE auth_members ALTER COLUMN joined_at SET DEFAULT now();
ALTER TABLE auth_organizations ADD COLUMN member_limit INTEGER CHECK(member_limit BETWEEN 1 AND 100000),
 ADD COLUMN invitation_limit INTEGER CHECK(invitation_limit BETWEEN 1 AND 100000);
ALTER TABLE auth_invitations ADD COLUMN cancelled_at TIMESTAMPTZ, ADD COLUMN rejected_at TIMESTAMPTZ,
 ADD COLUMN delivery_status TEXT NOT NULL DEFAULT 'unknown' CHECK(delivery_status IN ('unknown','sent','failed','not_configured')),
 ADD COLUMN delivery_attempted_at TIMESTAMPTZ, ADD COLUMN inviter_name TEXT;
ALTER TABLE auth_invitations ALTER COLUMN invited_by DROP NOT NULL;
ALTER TABLE auth_invitations DROP CONSTRAINT auth_invitations_invited_by_fkey;
ALTER TABLE auth_invitations ADD CONSTRAINT auth_invitations_invited_by_fkey FOREIGN KEY(invited_by) REFERENCES auth_users(id) ON DELETE SET NULL;
UPDATE auth_invitations i SET inviter_name=(SELECT name FROM auth_users WHERE id=i.invited_by);
CREATE FUNCTION ferrix_inviter_snapshot() RETURNS trigger LANGUAGE plpgsql AS $$
BEGIN
 NEW.inviter_name := COALESCE((SELECT name FROM auth_users WHERE id=NEW.invited_by),NEW.inviter_name);
 RETURN NEW;
END $$;
CREATE TRIGGER invitations_inviter_snapshot BEFORE INSERT OR UPDATE OF invited_by ON auth_invitations
 FOR EACH ROW EXECUTE FUNCTION ferrix_inviter_snapshot();
CREATE INDEX auth_invitations_history ON auth_invitations(organization_id,created_at DESC,id DESC);
