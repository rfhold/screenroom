GRANT USAGE ON SCHEMA identity TO screenroom_user;
GRANT SELECT,INSERT,UPDATE,DELETE ON ALL TABLES IN SCHEMA identity TO screenroom_user;
GRANT USAGE ON SCHEMA catalog,ingest,search,media TO screenroom_catalog,screenroom_worker;
GRANT SELECT ON ALL TABLES IN SCHEMA catalog,ingest,search,media TO screenroom_catalog,screenroom_worker;
GRANT INSERT,UPDATE ON catalog.sources,catalog.libraries TO screenroom_catalog;
GRANT INSERT,UPDATE ON ingest.policies,ingest.runs,ingest.jobs TO screenroom_catalog;
GRANT INSERT,DELETE ON search.result_snapshots TO screenroom_catalog;
GRANT INSERT,UPDATE,DELETE ON ALL TABLES IN SCHEMA catalog,ingest,search,media TO screenroom_worker;
REVOKE ALL ON FUNCTION ingest.immutable_snapshot() FROM PUBLIC;
