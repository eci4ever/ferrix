-- Pre-authentication proofs are browser-bound, short lived and never full sessions.
CREATE TABLE auth_login_challenges (
    token_hash TEXT PRIMARY KEY,
    user_id UUID NOT NULL REFERENCES auth_users(id) ON DELETE CASCADE,
    credential_hash TEXT NOT NULL,
    factor_hash TEXT NOT NULL,
    attempts INTEGER NOT NULL DEFAULT 0 CHECK (attempts BETWEEN 0 AND 5),
    expires_at TIMESTAMPTZ NOT NULL DEFAULT now() + interval '5 minutes'
);
CREATE INDEX auth_login_challenges_expiry_idx ON auth_login_challenges(expires_at);
CREATE INDEX auth_login_challenges_user_idx ON auth_login_challenges(user_id);
