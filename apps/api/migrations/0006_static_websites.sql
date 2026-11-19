CREATE TABLE static_websites (
    id UUID PRIMARY KEY,
    environment_id UUID NOT NULL REFERENCES environments(id) ON DELETE CASCADE,
    name TEXT NOT NULL CHECK (char_length(name) BETWEEN 1 AND 80),
    domain TEXT UNIQUE,
    active_release_id UUID,
    created_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    UNIQUE (environment_id, name)
);
CREATE TABLE static_releases (
    site_id UUID NOT NULL REFERENCES static_websites(id) ON DELETE CASCADE,
    id UUID NOT NULL,
    created_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    bytes INTEGER NOT NULL CHECK (bytes BETWEEN 0 AND 8388608),
    PRIMARY KEY (site_id, id)
);
ALTER TABLE static_websites ADD FOREIGN KEY (id, active_release_id)
    REFERENCES static_releases(site_id, id) DEFERRABLE INITIALLY DEFERRED;
CREATE TABLE static_files (
    site_id UUID NOT NULL,
    release_id UUID NOT NULL,
    path TEXT NOT NULL,
    content BYTEA NOT NULL,
    PRIMARY KEY (site_id, release_id, path),
    FOREIGN KEY (site_id, release_id) REFERENCES static_releases(site_id, id) ON DELETE CASCADE
);
