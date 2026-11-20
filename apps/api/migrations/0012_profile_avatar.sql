ALTER TABLE auth_users ADD COLUMN avatar TEXT NOT NULL DEFAULT 'initials'
CHECK (avatar IN ('initials','🌿','🚀','🐱','🌊','⭐'));
