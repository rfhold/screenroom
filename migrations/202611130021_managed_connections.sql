CREATE SCHEMA connections;
CREATE TABLE connections.jellyfin (
 id uuid PRIMARY KEY,
 source_id uuid NOT NULL UNIQUE REFERENCES catalog.sources(id),
 server_id text NOT NULL UNIQUE CHECK(length(server_id) BETWEEN 1 AND 200),
 server_name text NOT NULL CHECK(length(server_name) BETWEEN 1 AND 256),
 server_version text NOT NULL CHECK(length(server_version) BETWEEN 1 AND 100),
 base_url text NOT NULL UNIQUE CHECK(length(base_url) BETWEEN 1 AND 2048),
 credential_ref uuid NOT NULL UNIQUE,
 revision text NOT NULL CHECK(revision ~ '^[0-9a-f]{64}$'),
 generation bigint NOT NULL DEFAULT 1 CHECK(generation > 0),
 indexing_profile_id text NOT NULL CHECK(length(indexing_profile_id) BETWEEN 1 AND 200),
 created_at timestamptz NOT NULL DEFAULT clock_timestamp()
);
CREATE TABLE connections.save_receipts (
 request_id uuid PRIMARY KEY,
 fingerprint bytea NOT NULL CHECK(octet_length(fingerprint)=32),
 connection_id uuid NOT NULL REFERENCES connections.jellyfin(id),
 result jsonb NOT NULL,
 created_at timestamptz NOT NULL DEFAULT clock_timestamp()
);
DO $$ BEGIN
 IF NOT EXISTS (SELECT FROM pg_roles WHERE rolname='screenroom_operator') THEN
  CREATE ROLE screenroom_operator NOLOGIN;
 END IF;
END $$;
GRANT screenroom_catalog TO screenroom_operator;
GRANT USAGE ON SCHEMA connections TO screenroom_operator,screenroom_user,screenroom_worker;
GRANT SELECT ON connections.jellyfin TO screenroom_user,screenroom_worker;
GRANT SELECT,INSERT ON connections.jellyfin,connections.save_receipts TO screenroom_operator;
GRANT USAGE ON SCHEMA identity TO screenroom_operator;
GRANT SELECT ON identity.users TO screenroom_operator;
GRANT UPDATE(updated_at) ON identity.users TO screenroom_operator;
GRANT SELECT,INSERT,UPDATE ON identity.jellyfin_user_links TO screenroom_operator;
CREATE FUNCTION connections.immutable_connection() RETURNS trigger LANGUAGE plpgsql AS $$
BEGIN
 RAISE EXCEPTION 'managed connection replacement is not supported' USING ERRCODE='55000';
END $$;
CREATE TRIGGER immutable_connection BEFORE UPDATE ON connections.jellyfin
 FOR EACH ROW EXECUTE FUNCTION connections.immutable_connection();
CREATE FUNCTION connections.invalidate_personal_generation() RETURNS trigger
 LANGUAGE plpgsql SECURITY DEFINER SET search_path=pg_catalog AS $$
BEGIN
 IF NEW.configuration IS DISTINCT FROM OLD.configuration OR NEW.state IS DISTINCT FROM OLD.state THEN
  UPDATE identity.users u SET updated_at=GREATEST(clock_timestamp(),u.updated_at+interval '1 microsecond')
   WHERE EXISTS(SELECT FROM identity.jellyfin_user_links l WHERE l.user_id=u.id AND l.source_id=NEW.source_key);
 END IF;
 RETURN NEW;
END $$;
REVOKE ALL ON FUNCTION connections.invalidate_personal_generation() FROM PUBLIC;
CREATE TRIGGER managed_source_personal_generation AFTER UPDATE OF configuration,state ON catalog.sources
 FOR EACH ROW EXECUTE FUNCTION connections.invalidate_personal_generation();
