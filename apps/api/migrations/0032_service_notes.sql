ALTER TABLE applications ADD COLUMN notes TEXT NOT NULL DEFAULT '' CHECK (octet_length(notes) <= 32768), ADD COLUMN notes_revision BIGINT NOT NULL DEFAULT 0 CHECK (notes_revision >= 0);
