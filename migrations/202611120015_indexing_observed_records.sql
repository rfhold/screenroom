ALTER TABLE ingest.indexing_runs ADD COLUMN observed_records bigint NOT NULL DEFAULT 0 CHECK (observed_records >= 0);
UPDATE ingest.indexing_runs ir SET observed_records=r.items FROM ingest.runs r WHERE r.id=ir.run_id;
