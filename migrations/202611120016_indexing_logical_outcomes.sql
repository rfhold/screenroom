CREATE TABLE ingest.request_work_units (
 job_id uuid PRIMARY KEY REFERENCES queue.jobs(id),
 unit bigint NOT NULL DEFAULT 0 CHECK(unit>=0)
);
ALTER TABLE ingest.request_receipts ADD COLUMN work_unit bigint;
CREATE TABLE ingest.request_resolutions (
 receipt_id uuid PRIMARY KEY REFERENCES ingest.request_receipts(id),
 reason text NOT NULL CHECK(reason IN ('successful_replay','full_scan_fallback')),
 resolved_at timestamptz NOT NULL DEFAULT clock_timestamp()
);
GRANT SELECT,INSERT,UPDATE ON ingest.request_work_units TO screenroom_catalog,screenroom_worker;
GRANT SELECT,INSERT ON ingest.request_resolutions TO screenroom_catalog,screenroom_worker;
