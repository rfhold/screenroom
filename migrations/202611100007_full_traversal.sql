ALTER TABLE ingest.jobs ADD COLUMN window_requests bigint NOT NULL DEFAULT 0 CHECK(window_requests>=0);
ALTER TABLE ingest.checkpoints ADD COLUMN observed_run_id uuid REFERENCES ingest.runs(id);
CREATE TABLE ingest.source_cooldowns (
 source_id uuid PRIMARY KEY REFERENCES catalog.sources(id), until_at timestamptz NOT NULL DEFAULT now()
);
CREATE TABLE ingest.request_receipts (
 id uuid PRIMARY KEY DEFAULT gen_random_uuid(), run_id uuid NOT NULL REFERENCES ingest.runs(id),
 job_id uuid NOT NULL REFERENCES ingest.jobs(id), fence bigint NOT NULL,
 operation text NOT NULL CHECK(operation IN ('discovery','page','ancestor','collection','artwork','embedding')),
 outcome text NOT NULL DEFAULT 'unknown' CHECK(outcome IN ('unknown','success','retryable','permanent','unresolved')),
 is_request boolean NOT NULL DEFAULT true,
 operation_key text CHECK(length(operation_key) BETWEEN 1 AND 400),
 error_class text CHECK(error_class IN ('throttled','transport','timeout','unavailable','invalid','unauthorized')),
 reserved_at timestamptz NOT NULL DEFAULT now()
);
CREATE INDEX request_receipts_run ON ingest.request_receipts(run_id,outcome);
CREATE UNIQUE INDEX pending_operation_identity ON ingest.request_receipts(run_id,operation,operation_key) WHERE operation_key IS NOT NULL;
CREATE TABLE ingest.primary_receipts (
 run_id uuid NOT NULL REFERENCES ingest.runs(id), source_id uuid NOT NULL REFERENCES catalog.sources(id),
 external_id text NOT NULL CHECK(length(external_id) BETWEEN 1 AND 200),
 PRIMARY KEY(run_id,external_id),
 FOREIGN KEY(source_id,external_id) REFERENCES catalog.source_items(source_id,external_id)
);
CREATE TABLE ingest.item_observations (
 run_id uuid NOT NULL REFERENCES ingest.runs(id), source_id uuid NOT NULL REFERENCES catalog.sources(id),
 external_id text NOT NULL, library_external_id text NOT NULL,
 PRIMARY KEY(run_id,external_id,library_external_id),
 FOREIGN KEY(source_id,external_id) REFERENCES catalog.source_items(source_id,external_id),
 FOREIGN KEY(source_id,library_external_id) REFERENCES catalog.libraries(source_id,external_id)
);
ALTER TABLE catalog.items DROP CONSTRAINT items_kind_check;
ALTER TABLE catalog.items ADD CONSTRAINT items_kind_check CHECK(kind IN ('movie','series','season','episode','collection'));
ALTER TABLE catalog.items ADD CONSTRAINT collection_root CHECK(kind<>'collection' OR parent_id IS NULL);
CREATE TABLE catalog.collection_members (
 source_id uuid NOT NULL REFERENCES catalog.sources(id), container_id uuid NOT NULL, member_id uuid NOT NULL,
 ordinal integer CHECK(ordinal>=0), observed_run_id uuid NOT NULL REFERENCES ingest.runs(id),
 observation_receipt_id uuid NOT NULL REFERENCES ingest.request_receipts(id),
 PRIMARY KEY(container_id,member_id), CHECK(container_id<>member_id),
 FOREIGN KEY(container_id,source_id) REFERENCES catalog.source_items(id,source_id),
 FOREIGN KEY(member_id,source_id) REFERENCES catalog.source_items(id,source_id)
);
GRANT SELECT ON ingest.source_cooldowns,ingest.request_receipts,ingest.primary_receipts,ingest.item_observations,catalog.collection_members TO screenroom_catalog,screenroom_worker;
GRANT INSERT,UPDATE ON ingest.source_cooldowns,ingest.request_receipts,ingest.primary_receipts,ingest.item_observations,catalog.collection_members TO screenroom_worker;
GRANT INSERT,UPDATE ON ingest.source_cooldowns TO screenroom_catalog;
