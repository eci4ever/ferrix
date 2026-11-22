CREATE TABLE application_domains (
 id UUID PRIMARY KEY,
 application_id UUID NOT NULL REFERENCES applications(id) ON DELETE CASCADE,
 hostname TEXT NOT NULL CHECK (length(hostname) BETWEEN 3 AND 253),
 verification_token TEXT NOT NULL,
 verified_at TIMESTAMPTZ,
 dns_status TEXT NOT NULL DEFAULT 'pending' CHECK (dns_status IN ('pending','ready','mismatch','unavailable')),
 tls_status TEXT NOT NULL DEFAULT 'pending' CHECK (tls_status IN ('pending','active','unavailable','disabled')),
 certificate_expires_at TIMESTAMPTZ,
 message TEXT,
 checked_at TIMESTAMPTZ,
 created_at TIMESTAMPTZ NOT NULL DEFAULT now(),
 UNIQUE(application_id,hostname)
);
CREATE UNIQUE INDEX application_domains_verified_host ON application_domains(hostname) WHERE verified_at IS NOT NULL;
