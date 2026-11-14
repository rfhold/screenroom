CREATE SCHEMA personal;
CREATE TABLE personal.watched_snapshots (
 user_id uuid NOT NULL REFERENCES identity.users(id),
 source_id uuid NOT NULL REFERENCES catalog.sources(id),
 generation uuid NOT NULL UNIQUE,
 binding_revision text NOT NULL,
 fetched_at timestamptz NOT NULL,
 complete boolean NOT NULL CHECK (complete),
 PRIMARY KEY(user_id,source_id)
);
CREATE TABLE personal.watched_observations (
 generation uuid NOT NULL REFERENCES personal.watched_snapshots(generation) ON DELETE CASCADE,
 external_item_id text NOT NULL CHECK(length(external_item_id) BETWEEN 1 AND 200),
 watched boolean,
 PRIMARY KEY(generation,external_item_id)
);
GRANT USAGE ON SCHEMA personal TO screenroom_user,screenroom_catalog;
GRANT SELECT ON ALL TABLES IN SCHEMA personal TO screenroom_user,screenroom_catalog;
GRANT INSERT,DELETE ON ALL TABLES IN SCHEMA personal TO screenroom_user;
GRANT USAGE ON SCHEMA identity TO screenroom_catalog;
GRANT SELECT ON identity.users TO screenroom_catalog;
GRANT USAGE ON SCHEMA catalog TO screenroom_user;
GRANT SELECT ON catalog.sources,catalog.libraries TO screenroom_user;
