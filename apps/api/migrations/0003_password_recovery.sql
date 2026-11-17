CREATE TABLE auth_password_resets (
 user_id UUID PRIMARY KEY REFERENCES auth_users(id) ON DELETE CASCADE,
 token_hash TEXT NOT NULL UNIQUE,
 expires_at TIMESTAMPTZ NOT NULL DEFAULT now()+interval '15 minutes'
);
