CREATE SCHEMA playback;
DO $$ BEGIN
 IF NOT EXISTS (SELECT FROM pg_roles WHERE rolname='screenroom_playback') THEN
  CREATE ROLE screenroom_playback NOLOGIN NOINHERIT;
 ELSIF EXISTS(SELECT FROM pg_roles WHERE rolname='screenroom_playback'
   AND (rolcanlogin OR rolsuper OR rolcreatedb OR rolcreaterole OR rolreplication OR rolbypassrls))
   OR EXISTS(SELECT FROM pg_auth_members WHERE member=(SELECT oid FROM pg_roles WHERE rolname='screenroom_playback')) THEN
  RAISE EXCEPTION 'existing playback role is not narrow';
 END IF;
END $$;

CREATE TABLE playback.freshness (
 user_id uuid NOT NULL REFERENCES identity.users(id),
 source_id uuid NOT NULL REFERENCES catalog.sources(id),
 binding_revision text NOT NULL,
 epoch bigint NOT NULL DEFAULT 0 CHECK(epoch>=0),
 updated_at timestamptz NOT NULL DEFAULT clock_timestamp(),
 PRIMARY KEY(user_id,source_id,binding_revision)
);
ALTER TABLE personal.watched_snapshots ADD COLUMN playback_epoch bigint NOT NULL DEFAULT 0 CHECK(playback_epoch>=0);

CREATE TABLE playback.sessions (
 id uuid PRIMARY KEY,
 user_id uuid NOT NULL REFERENCES identity.users(id),
 public_item_id text NOT NULL,
 source_item_id uuid NOT NULL REFERENCES catalog.source_items(id),
 source_id uuid NOT NULL REFERENCES catalog.sources(id),
 source_key text NOT NULL,
 scope jsonb NOT NULL,
 external_item_id text NOT NULL,
 connection_id uuid NOT NULL REFERENCES connections.jellyfin(id),
 connection_generation bigint NOT NULL CHECK(connection_generation>0),
 binding_revision text NOT NULL,
 remote_profile_id text NOT NULL,
 device_id text NOT NULL UNIQUE,
 play_session_id text,
 credential_ref uuid NOT NULL UNIQUE,
 replay_ref uuid NOT NULL UNIQUE,
 control_hash bytea NOT NULL CHECK(octet_length(control_hash)=32),
 media_hash bytea CHECK(media_hash IS NULL OR octet_length(media_hash)=32),
 state text NOT NULL CHECK(state IN ('preparing','ready','playing','paused','stopping','stopped','expired','failed','uncertain')),
 transport_revision bigint NOT NULL DEFAULT 0 CHECK(transport_revision BETWEEN 0 AND 4294967295),
 last_sequence bigint NOT NULL DEFAULT 0 CHECK(last_sequence BETWEEN 0 AND 4294967295),
 capabilities jsonb NOT NULL,
 tracks jsonb NOT NULL DEFAULT '[]',
 transport jsonb,
 lease_until timestamptz NOT NULL DEFAULT clock_timestamp()+interval '90 seconds',
 absolute_until timestamptz NOT NULL DEFAULT clock_timestamp()+interval '12 hours',
 paused_at timestamptz,
 created_at timestamptz NOT NULL DEFAULT clock_timestamp(),
 updated_at timestamptz NOT NULL DEFAULT clock_timestamp()
);
CREATE UNIQUE INDEX playback_duplicate_active ON playback.sessions(connection_id,remote_profile_id,external_item_id)
 WHERE state IN ('preparing','ready','playing','paused','stopping');
CREATE INDEX playback_session_expiry ON playback.sessions(lease_until);

CREATE TABLE playback.operations (
 id uuid PRIMARY KEY,
 session_id uuid NOT NULL REFERENCES playback.sessions(id),
 user_id uuid NOT NULL,
 binding_revision text NOT NULL,
 kind text NOT NULL CHECK(kind IN ('create','update','event','stop')),
 request_id uuid,
 sequence bigint CHECK(sequence BETWEEN 1 AND 4294967295),
 transport_revision bigint NOT NULL CHECK(transport_revision BETWEEN 0 AND 4294967295),
 body jsonb NOT NULL CHECK(octet_length(body::text)<=65536),
 replay_ref uuid UNIQUE,
 status text NOT NULL DEFAULT 'pending' CHECK(status IN ('pending','confirmed','uncertain','failed')),
 claim uuid NOT NULL,
 dispatched boolean NOT NULL DEFAULT false,
 result jsonb,
 created_at timestamptz NOT NULL DEFAULT clock_timestamp(),
 finished_at timestamptz,
 UNIQUE(user_id,binding_revision,kind,request_id),
 UNIQUE(session_id,sequence),
 CHECK((kind='event')=(sequence IS NOT NULL)),
 CHECK((kind='event')=(request_id IS NULL)),
 CHECK((kind IN ('create','update'))=(replay_ref IS NOT NULL))
);
CREATE UNIQUE INDEX playback_one_pending ON playback.operations(session_id) WHERE status IN ('pending','uncertain');
CREATE TABLE playback.effects (
 operation_id uuid PRIMARY KEY REFERENCES playback.operations(id),
 user_id uuid NOT NULL REFERENCES identity.users(id),
 source_id uuid NOT NULL REFERENCES catalog.sources(id),
 binding_revision text NOT NULL,
 remote_profile_id text NOT NULL,
 status text NOT NULL DEFAULT 'pending' CHECK(status IN ('pending','unknown','completed','no_effect')),
 evidence text CHECK(evidence IN ('remote_completion','controlled_quiescence','not_dispatched')),
 updated_at timestamptz NOT NULL DEFAULT clock_timestamp(),
 CHECK((status IN ('pending','unknown'))=(evidence IS NULL))
);
CREATE INDEX playback_pending_effects ON playback.effects(user_id,source_id,binding_revision) WHERE status IN ('pending','unknown');
CREATE INDEX playback_pending_profiles ON playback.effects(source_id,remote_profile_id) WHERE status IN ('pending','unknown');
CREATE TABLE playback.resources (
 id uuid PRIMARY KEY,
 session_id uuid NOT NULL REFERENCES playback.sessions(id),
 transport_revision bigint NOT NULL CHECK(transport_revision BETWEEN 1 AND 4294967295),
 source_item_id uuid NOT NULL REFERENCES catalog.source_items(id),
 media_source_id text NOT NULL,
 private_mapping jsonb NOT NULL CHECK(octet_length(private_mapping::text)<=16384),
 UNIQUE(session_id,transport_revision,id)
);
CREATE TABLE playback.cleanup (
 id uuid PRIMARY KEY,
 session_id uuid NOT NULL REFERENCES playback.sessions(id),
 device_id text NOT NULL,
 play_session_id text,
 credential_ref uuid NOT NULL,
 revoke_device boolean NOT NULL,
 state text NOT NULL DEFAULT 'pending' CHECK(state IN ('pending','claimed','complete','failed')),
 attempts integer NOT NULL DEFAULT 0 CHECK(attempts BETWEEN 0 AND 5),
 claim uuid,
 claim_until timestamptz,
 available_at timestamptz NOT NULL DEFAULT clock_timestamp(),
 error_code text CHECK(error_code IN ('upstream_unavailable','identity_changed','attempts_exhausted')),
 created_at timestamptz NOT NULL DEFAULT clock_timestamp(),
 finished_at timestamptz
);
CREATE UNIQUE INDEX playback_device_cleanup ON playback.cleanup(session_id) WHERE revoke_device;
CREATE TABLE playback.retired_credentials (
 reference uuid PRIMARY KEY,
 retired_at timestamptz NOT NULL DEFAULT clock_timestamp()
);

CREATE FUNCTION playback.lock_user(target uuid) RETURNS text
 LANGUAGE sql SECURITY DEFINER SET search_path=pg_catalog AS $$
 SELECT CASE WHEN state='enabled' THEN updated_at::text END FROM identity.users WHERE id=target FOR NO KEY UPDATE
$$;
REVOKE ALL ON FUNCTION playback.lock_user(uuid) FROM PUBLIC;
GRANT EXECUTE ON FUNCTION playback.lock_user(uuid) TO screenroom_playback;

CREATE FUNCTION playback.revoke_user_sessions() RETURNS trigger
 LANGUAGE plpgsql SECURITY DEFINER SET search_path=pg_catalog AS $$
BEGIN
 IF NEW.updated_at IS DISTINCT FROM OLD.updated_at OR NEW.state IS DISTINCT FROM OLD.state THEN
  UPDATE playback.sessions SET state='stopping',media_hash=NULL,updated_at=clock_timestamp()
   WHERE user_id=NEW.id AND state IN ('preparing','ready','playing','paused');
 END IF;
 RETURN NEW;
END $$;
REVOKE ALL ON FUNCTION playback.revoke_user_sessions() FROM PUBLIC;
CREATE TRIGGER playback_user_fence AFTER UPDATE OF updated_at,state ON identity.users
 FOR EACH ROW EXECUTE FUNCTION playback.revoke_user_sessions();

GRANT USAGE ON SCHEMA playback,identity,catalog,connections TO screenroom_playback;
GRANT SELECT ON identity.users,identity.jellyfin_user_links TO screenroom_playback;
GRANT SELECT ON catalog.sources,catalog.items,catalog.source_items,catalog.libraries,catalog.library_items,catalog.external_ids TO screenroom_playback;
GRANT SELECT(id,source_id,server_id,server_name,base_url,credential_ref,revision,generation) ON connections.jellyfin TO screenroom_playback;
GRANT SELECT,INSERT,UPDATE,DELETE ON ALL TABLES IN SCHEMA playback TO screenroom_playback;
GRANT USAGE ON SCHEMA playback TO screenroom_user,screenroom_catalog;
GRANT SELECT ON playback.freshness TO screenroom_user,screenroom_catalog;
CREATE FUNCTION playback.has_pending_effect(target_user uuid,target_source uuid,target_revision text) RETURNS boolean
 LANGUAGE sql STABLE SECURITY DEFINER SET search_path=pg_catalog AS $$
 SELECT EXISTS(SELECT FROM playback.effects e WHERE e.source_id=target_source AND e.status IN ('pending','unknown')
  AND (e.user_id=target_user AND e.binding_revision=target_revision OR e.remote_profile_id IN
   (SELECT l.jellyfin_user_id FROM identity.jellyfin_user_links l JOIN catalog.sources s ON s.source_key=l.source_id
    WHERE l.user_id=target_user AND s.id=target_source)))
$$;
REVOKE ALL ON FUNCTION playback.has_pending_effect(uuid,uuid,text) FROM PUBLIC;
GRANT EXECUTE ON FUNCTION playback.has_pending_effect(uuid,uuid,text) TO screenroom_user,screenroom_catalog;
