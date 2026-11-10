ALTER TABLE ingest.policies ADD CONSTRAINT policy_source_identity UNIQUE(id,source_id);
ALTER TABLE ingest.runs ADD COLUMN source_id uuid;
UPDATE ingest.runs r SET source_id=p.source_id FROM ingest.policies p WHERE p.id=r.policy_id;
ALTER TABLE ingest.runs ALTER COLUMN source_id SET NOT NULL;
ALTER TABLE ingest.runs ADD CONSTRAINT run_policy_source FOREIGN KEY(policy_id,source_id) REFERENCES ingest.policies(id,source_id);
CREATE UNIQUE INDEX one_active_source_run ON ingest.runs(source_id) WHERE state IN ('running','paused');
CREATE OR REPLACE FUNCTION ingest.immutable_snapshot() RETURNS trigger LANGUAGE plpgsql AS $$ BEGIN
 IF NEW.policy_snapshot IS DISTINCT FROM OLD.policy_snapshot OR NEW.selection_snapshot IS DISTINCT FROM OLD.selection_snapshot OR NEW.policy_version<>OLD.policy_version OR NEW.policy_id<>OLD.policy_id OR NEW.source_id<>OLD.source_id THEN
 RAISE EXCEPTION 'run snapshot is immutable'; END IF; RETURN NEW; END $$;

DO $$ BEGIN
 IF EXISTS(SELECT FROM identity.jellyfin_user_links l LEFT JOIN catalog.sources s ON s.id=l.source_id WHERE s.id IS NULL OR length(s.source_key) NOT BETWEEN 1 AND 200) THEN
  RAISE EXCEPTION 'Jellyfin user link has no bounded stable source key';
 END IF;
END $$;
ALTER TABLE identity.jellyfin_user_links ALTER COLUMN source_id TYPE text USING source_id::text;
UPDATE identity.jellyfin_user_links l SET source_id=s.source_key FROM catalog.sources s WHERE l.source_id=s.id::text;
ALTER TABLE identity.jellyfin_user_links ADD CONSTRAINT bounded_source_key CHECK(length(source_id) BETWEEN 1 AND 200);
