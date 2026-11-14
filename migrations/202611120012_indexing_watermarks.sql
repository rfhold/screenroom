ALTER TABLE ingest.indexing_runs
 ADD COLUMN lower_watermark timestamptz,
 ADD COLUMN candidate_watermark timestamptz,
 ADD COLUMN settled_at timestamptz,
 ADD COLUMN fallback_reason text;
CREATE TABLE ingest.indexing_progress (
 policy_id uuid PRIMARY KEY REFERENCES ingest.policies(id),
 policy_version bigint NOT NULL,
 watermark timestamptz,
 next_reconcile_at timestamptz NOT NULL DEFAULT clock_timestamp()
);
GRANT SELECT,INSERT,UPDATE ON ingest.indexing_progress TO screenroom_catalog,screenroom_worker;
GRANT UPDATE ON ingest.indexing_runs TO screenroom_catalog,screenroom_worker;
