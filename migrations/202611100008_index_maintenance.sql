CREATE TABLE ingest.maintenance_starts (
 request_id uuid PRIMARY KEY, run_id uuid NOT NULL UNIQUE REFERENCES ingest.runs(id),
 policy_id uuid NOT NULL REFERENCES ingest.policies(id), request jsonb NOT NULL,
 trigger text NOT NULL CHECK(trigger IN ('manual','scheduled')), as_of timestamptz NOT NULL DEFAULT now()
);
CREATE TABLE ingest.maintenance_candidates (
 run_id uuid NOT NULL REFERENCES ingest.runs(id), source_id uuid NOT NULL REFERENCES catalog.sources(id),
 source_item_id uuid NOT NULL, external_id text NOT NULL, public_id text NOT NULL,
 kind text NOT NULL CHECK(kind IN ('movie','series')),
 state text NOT NULL DEFAULT 'selected' CHECK(state IN ('selected','checked','skipped_ready','staged','unavailable','failed')),
 code text CHECK(code IN ('ready','no_poster','unavailable','invalid','unauthorized','transport','timeout')),
 source_artwork_id text, selected_at timestamptz NOT NULL DEFAULT now(), updated_at timestamptz NOT NULL DEFAULT now(),
 PRIMARY KEY(run_id,source_item_id),
 FOREIGN KEY(source_item_id,source_id) REFERENCES catalog.source_items(id,source_id)
);
CREATE INDEX maintenance_candidate_history ON ingest.maintenance_candidates(source_item_id,updated_at DESC);
GRANT SELECT ON ingest.maintenance_starts,ingest.maintenance_candidates TO screenroom_catalog,screenroom_worker;
GRANT INSERT,UPDATE ON ingest.maintenance_starts,ingest.maintenance_candidates TO screenroom_catalog,screenroom_worker;
