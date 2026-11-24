CREATE TABLE resource_quotas (
 scope TEXT NOT NULL CHECK(scope IN ('host','organization','project')),
 scope_id UUID NOT NULL,
 memory_mib BIGINT CHECK(memory_mib>=0),
 cpu_centi BIGINT CHECK(cpu_centi>=0),
 services BIGINT CHECK(services>=0),
 databases BIGINT CHECK(databases>=0),
 deployments BIGINT CHECK(deployments>=0),
 builds BIGINT CHECK(builds>=0),
 storage_bytes BIGINT CHECK(storage_bytes>=0),
 reserved_memory_mib BIGINT NOT NULL DEFAULT 0 CHECK(reserved_memory_mib>=0),
 reserved_cpu_centi BIGINT NOT NULL DEFAULT 0 CHECK(reserved_cpu_centi>=0),
 updated_by UUID REFERENCES auth_users(id) ON DELETE SET NULL,
 updated_at TIMESTAMPTZ NOT NULL DEFAULT now(),
 PRIMARY KEY(scope,scope_id),
 CHECK(scope='host' OR (reserved_memory_mib=0 AND reserved_cpu_centi=0)),
 CHECK(scope<>'host' OR scope_id='00000000-0000-0000-0000-000000000000')
);
CREATE TABLE storage_volume_usage (
 volume_name TEXT PRIMARY KEY,
 application_id UUID REFERENCES applications(id) ON DELETE SET NULL,
 project_id UUID REFERENCES projects(id) ON DELETE SET NULL,
 organization_id UUID REFERENCES auth_organizations(id) ON DELETE SET NULL,
 size_bytes BIGINT CHECK(size_bytes>=0),
 checked_at TIMESTAMPTZ NOT NULL DEFAULT now()
);
CREATE VIEW service_storage_usage AS SELECT application_id,sum(size_bytes)::bigint AS size_bytes,min(checked_at) AS checked_at FROM storage_volume_usage WHERE application_id IS NOT NULL GROUP BY application_id;
CREATE TABLE quota_storage_alerts (
 scope TEXT NOT NULL,
 scope_id UUID NOT NULL,
 active BOOLEAN NOT NULL DEFAULT false,
 PRIMARY KEY(scope,scope_id),
 FOREIGN KEY(scope,scope_id) REFERENCES resource_quotas(scope,scope_id) ON DELETE CASCADE
);
