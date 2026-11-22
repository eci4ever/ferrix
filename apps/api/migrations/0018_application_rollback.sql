ALTER TABLE deployments ADD COLUMN rollback_of UUID REFERENCES deployments(id) ON DELETE SET NULL;
ALTER TABLE deployments ADD COLUMN pinned_image_id TEXT;
