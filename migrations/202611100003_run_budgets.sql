ALTER TABLE ingest.runs
 ADD COLUMN budget_items bigint NOT NULL DEFAULT 0 CHECK(budget_items>=0),
 ADD COLUMN budget_requests bigint NOT NULL DEFAULT 0 CHECK(budget_requests>=0);
