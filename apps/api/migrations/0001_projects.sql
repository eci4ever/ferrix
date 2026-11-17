CREATE TABLE projects (
    id UUID PRIMARY KEY,
    name TEXT NOT NULL CHECK (char_length(name) BETWEEN 1 AND 80),
    description TEXT NOT NULL DEFAULT '' CHECK (char_length(description) <= 500),
    created_at TIMESTAMPTZ NOT NULL DEFAULT now()
);
CREATE INDEX projects_created_at_idx ON projects (created_at DESC);

