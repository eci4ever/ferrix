ALTER TABLE auth_users ADD COLUMN avatar_url TEXT CHECK (octet_length(avatar_url) <= 2048);
