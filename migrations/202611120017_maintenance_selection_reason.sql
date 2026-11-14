ALTER TABLE ingest.maintenance_candidates ADD COLUMN selection_reason text CHECK(selection_reason IN ('unknown_age','known_stale'));
