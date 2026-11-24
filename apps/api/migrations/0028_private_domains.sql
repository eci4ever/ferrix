ALTER TABLE application_domains ADD COLUMN route_kind TEXT NOT NULL DEFAULT 'direct' CHECK(route_kind IN ('direct','lan','tunnel'));
ALTER TABLE application_domains ADD COLUMN canonical BOOLEAN NOT NULL DEFAULT false;
ALTER TABLE application_domains ADD COLUMN http_status TEXT NOT NULL DEFAULT 'unverified' CHECK(http_status IN ('unverified','reachable','unavailable'));
ALTER TABLE application_domains ADD COLUMN tunnel_status TEXT NOT NULL DEFAULT 'not_applicable' CHECK(tunnel_status IN ('not_applicable','configuration_required','reachable','unverified'));
CREATE UNIQUE INDEX application_domains_canonical ON application_domains(application_id) WHERE canonical;
