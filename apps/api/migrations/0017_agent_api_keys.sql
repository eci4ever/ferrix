CREATE TABLE auth_api_keys (
 id UUID PRIMARY KEY,
 user_id UUID NOT NULL,
 organization_id UUID NOT NULL,
 name TEXT NOT NULL CHECK(char_length(name) BETWEEN 1 AND 80),
 key_hash TEXT NOT NULL UNIQUE,
 prefix TEXT NOT NULL,
 scopes TEXT[] NOT NULL CHECK(scopes <@ ARRAY['read','manage','deploy','delete','secrets:read']::TEXT[] AND 'read'=ANY(scopes) AND cardinality(scopes) BETWEEN 1 AND 5),
 created_at TIMESTAMPTZ NOT NULL DEFAULT now(),
 expires_at TIMESTAMPTZ NOT NULL,
 revoked_at TIMESTAMPTZ,
 last_used_at TIMESTAMPTZ,
 rate_window TIMESTAMPTZ NOT NULL DEFAULT date_trunc('minute',now()),
 rate_count INTEGER NOT NULL DEFAULT 0,
 FOREIGN KEY(organization_id,user_id) REFERENCES auth_members(organization_id,user_id) ON DELETE CASCADE
);
CREATE INDEX auth_api_keys_owner_idx ON auth_api_keys(user_id,organization_id,created_at DESC);
CREATE TABLE auth_api_key_requests (
 id UUID PRIMARY KEY,
 key_id UUID REFERENCES auth_api_keys(id) ON DELETE SET NULL,
 organization_id UUID REFERENCES auth_organizations(id) ON DELETE CASCADE,
 user_id UUID REFERENCES auth_users(id) ON DELETE SET NULL,
 method TEXT NOT NULL,
 route TEXT NOT NULL,
 resource_id UUID,
 status SMALLINT NOT NULL CHECK(status BETWEEN 100 AND 599),
 ip_address TEXT NOT NULL,
 created_at TIMESTAMPTZ NOT NULL DEFAULT now(),
 expires_at TIMESTAMPTZ NOT NULL DEFAULT now()+interval '30 days'
);
CREATE INDEX auth_api_key_requests_key_idx ON auth_api_key_requests(key_id,created_at DESC);
CREATE INDEX auth_api_key_requests_expiry_idx ON auth_api_key_requests(expires_at);
