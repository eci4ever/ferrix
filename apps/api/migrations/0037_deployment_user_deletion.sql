-- Keep deployment history when its requesting account is deleted.
ALTER TABLE deployments DROP CONSTRAINT deployments_requested_by_fkey;
ALTER TABLE deployments ADD CONSTRAINT deployments_requested_by_fkey
    FOREIGN KEY (requested_by) REFERENCES auth_users(id) ON DELETE SET NULL;
