CREATE TABLE student_hosting_settings (
 singleton BOOLEAN PRIMARY KEY DEFAULT true CHECK(singleton),
 instance_id UUID NOT NULL DEFAULT gen_random_uuid(),
 enabled BOOLEAN NOT NULL DEFAULT false,
 registration_open BOOLEAN NOT NULL DEFAULT false,
 root_cipher BYTEA,
 server_state TEXT NOT NULL DEFAULT 'not_started',
 server_error TEXT,
 updated_at TIMESTAMPTZ NOT NULL DEFAULT now()
);
INSERT INTO student_hosting_settings(singleton) VALUES(true);
CREATE TABLE student_hostings (
 user_id UUID PRIMARY KEY REFERENCES auth_users(id) ON DELETE RESTRICT,
 organization_id UUID NOT NULL REFERENCES auth_organizations(id) ON DELETE RESTRICT,
 status TEXT NOT NULL DEFAULT 'pending' CHECK(status IN ('pending','queued','provisioning','active','failed','suspended','rejected')),
 database_name TEXT NOT NULL UNIQUE,
 database_user TEXT NOT NULL UNIQUE,
 password_cipher BYTEA NOT NULL,
 container_id TEXT,
 error TEXT,
 approved_by UUID REFERENCES auth_users(id),
 approved_at TIMESTAMPTZ,
 created_at TIMESTAMPTZ NOT NULL DEFAULT now()
);
