ALTER TABLE queue.jobs
 ADD COLUMN retry_base_seconds integer NOT NULL DEFAULT 5 CHECK(retry_base_seconds BETWEEN 1 AND 300),
 ADD COLUMN retry_cap_seconds integer NOT NULL DEFAULT 3600 CHECK(retry_cap_seconds BETWEEN retry_base_seconds AND 86400),
 ADD COLUMN retry_jitter_percent integer NOT NULL DEFAULT 25 CHECK(retry_jitter_percent BETWEEN 0 AND 100);

CREATE TABLE ingest.indexing_receipts (
 request_id uuid PRIMARY KEY,
 operation text NOT NULL CHECK(operation IN ('configure','start')),
 input jsonb NOT NULL,
 result jsonb NOT NULL,
 created_at timestamptz NOT NULL DEFAULT clock_timestamp()
);
CREATE TABLE ingest.indexing_runs (
 run_id uuid PRIMARY KEY REFERENCES ingest.runs(id),
 source_configuration jsonb NOT NULL,
 mode text NOT NULL CHECK(mode IN ('full','incremental','reconcile','repair')),
 admitted_at timestamptz NOT NULL DEFAULT clock_timestamp()
);
GRANT SELECT,INSERT ON ingest.indexing_receipts,ingest.indexing_runs TO screenroom_catalog,screenroom_worker;
