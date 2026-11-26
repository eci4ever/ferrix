CREATE TABLE php_classroom_applications (
    application_id uuid PRIMARY KEY REFERENCES applications(id) ON DELETE CASCADE,
    organization_id uuid NOT NULL REFERENCES auth_organizations(id),
    student_id uuid REFERENCES auth_users(id) ON DELETE SET NULL,
    database_service_id uuid NOT NULL REFERENCES applications(id) ON DELETE RESTRICT,
    version text NOT NULL CHECK(version='php-v1'),
    parameters jsonb NOT NULL,
    status text NOT NULL DEFAULT 'awaiting_database' CHECK(status IN ('awaiting_database','ready','failed')),
    message text NOT NULL DEFAULT 'Review source, then provision the managed database before deploying.',
    created_at timestamptz NOT NULL DEFAULT now()
);
