-- Installation configuration is encrypted using FERRIX_AUTH_KEY, with section-bound AAD.
CREATE TABLE platform_settings (
 section TEXT PRIMARY KEY CHECK (section IN ('timezone','email','social')),
 revision BIGINT NOT NULL DEFAULT 0 CHECK (revision >= 0),
 config_cipher BYTEA,
 updated_at TIMESTAMPTZ NOT NULL DEFAULT now()
);
