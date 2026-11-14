CREATE SCHEMA queue;
ALTER TABLE ingest.jobs SET SCHEMA queue;
ALTER TABLE queue.jobs RENAME COLUMN run_id TO scope_id;
ALTER TABLE queue.jobs DROP CONSTRAINT jobs_run_id_fkey;
ALTER TABLE queue.jobs DROP CONSTRAINT jobs_kind_check;
ALTER TABLE queue.jobs ADD COLUMN queue_name text NOT NULL DEFAULT 'indexing';
ALTER TABLE queue.jobs ADD COLUMN requirements text[] NOT NULL DEFAULT '{}';
ALTER TABLE queue.jobs ADD COLUMN redrive_generation integer NOT NULL DEFAULT 0 CHECK(redrive_generation>=0);
DROP INDEX queue.active_job_dedupe;
CREATE UNIQUE INDEX active_job_dedupe ON queue.jobs(queue_name,scope_id,kind,dedupe_key) WHERE state IN ('ready','leased');
CREATE TABLE queue.attempts (
 job_id uuid NOT NULL REFERENCES queue.jobs(id), fence bigint NOT NULL, owner text NOT NULL, generation integer NOT NULL DEFAULT 0,
 started_at timestamptz NOT NULL DEFAULT clock_timestamp(), finished_at timestamptz,
 outcome text NOT NULL DEFAULT 'running' CHECK(outcome IN ('running','complete','released','permanent','retryable','throttled','lease_expired','exhausted','paused','cancelled')),
 error_code text CHECK(error_code ~ '^[a-z_]{1,64}$'), PRIMARY KEY(job_id,fence)
);
CREATE TABLE queue.redrives (
 request_id uuid PRIMARY KEY, job_id uuid NOT NULL REFERENCES queue.jobs(id), generation integer NOT NULL,
 requested_at timestamptz NOT NULL DEFAULT clock_timestamp(), UNIQUE(job_id,generation)
);
CREATE TABLE ingest.run_redrives (
 request_id uuid PRIMARY KEY REFERENCES queue.redrives(request_id), run_id uuid NOT NULL REFERENCES ingest.runs(id),
 prior_state text NOT NULL, prior_finished_at timestamptz, prior_coverage jsonb NOT NULL
);
GRANT SELECT,INSERT ON ingest.run_redrives TO screenroom_catalog,screenroom_worker;
INSERT INTO queue.attempts(job_id,fence,owner) SELECT id,fence,lease_owner FROM queue.jobs WHERE state='leased';
ALTER TABLE ingest.runs ADD COLUMN embedding_model_id uuid REFERENCES search.embedding_models(id);
UPDATE ingest.runs r SET embedding_model_id=(SELECT m.id FROM queue.jobs j JOIN search.embedding_models m ON m.id::text=j.input->>'model_id' WHERE j.scope_id=r.id AND j.kind='embedding' ORDER BY j.id LIMIT 1);
UPDATE ingest.runs SET embedding_model_id=(SELECT id FROM search.embedding_models WHERE state<>'retired' ORDER BY (state='active') DESC,name,revision,id LIMIT 1) WHERE embedding_model_id IS NULL;
UPDATE queue.jobs j SET input=jsonb_set(j.input,'{input_hash}',to_jsonb(encode(d.content_hash,'hex'))) FROM ingest.runs r JOIN catalog.source_items si ON si.source_id=r.source_id JOIN search.documents d ON d.source_item_id=si.id WHERE j.scope_id=r.id AND j.kind='embedding' AND si.external_id=j.input->>'external_id';
-- Repair active cross-run publication dependencies while retaining the original job identities.
INSERT INTO queue.jobs(scope_id,kind,dedupe_key,input)
SELECT DISTINCT observation.run_id,'artwork',original.dedupe_key,original.input
FROM ingest.request_receipts observation JOIN ingest.runs r ON r.id=observation.run_id
JOIN media.artwork_generations g ON observation.operation_key='publication:'||g.artwork_id::text||':'||g.revision::text
JOIN media.artwork a ON a.id=g.artwork_id AND a.current_generation=g.revision
JOIN LATERAL (SELECT DISTINCT ON (dedupe_key) dedupe_key,input FROM queue.jobs j WHERE j.kind='artwork' AND j.input->>'artwork_id'=a.id::text AND j.input->>'revision'=g.revision::text ORDER BY dedupe_key,id) original ON true
WHERE observation.operation='artwork' AND NOT observation.is_request AND r.state IN ('running','paused','complete') AND g.state='pending'
AND NOT EXISTS(SELECT FROM queue.jobs own WHERE own.scope_id=r.id AND own.kind='artwork' AND own.dedupe_key=original.dedupe_key)
ON CONFLICT DO NOTHING;
UPDATE queue.jobs j SET requirements=ARRAY['adapter:'||COALESCE(s.configuration->>'adapter','unsupported')]
 || CASE WHEN s.configuration->>'adapter'='jellyfin' THEN ARRAY['source:'||s.source_key] ELSE '{}'::text[] END
 || CASE WHEN j.kind='embedding' THEN ARRAY['model:'||(j.input->>'model_id')] ELSE '{}'::text[] END
  FROM ingest.runs r JOIN catalog.sources s ON s.id=r.source_id WHERE j.scope_id=r.id;
UPDATE queue.jobs j SET state='failed',lease_owner=NULL,lease_until=NULL,fence=fence+1,error_code='invalid_model'
 WHERE j.kind='embedding' AND j.state IN ('ready','leased') AND NOT EXISTS(SELECT FROM search.embedding_models m WHERE m.id::text=j.input->>'model_id');
UPDATE queue.attempts a SET finished_at=clock_timestamp(),outcome='permanent',error_code='invalid_model'
 FROM queue.jobs j WHERE j.id=a.job_id AND j.state='failed' AND j.error_code='invalid_model' AND a.finished_at IS NULL;
UPDATE queue.jobs SET state='failed',lease_owner=NULL,lease_until=NULL,fence=fence+1,error_code='invalid_scope' WHERE scope_id IS NULL AND state IN ('ready','leased');
UPDATE queue.attempts a SET finished_at=clock_timestamp(),outcome='permanent',error_code='invalid_scope'
 FROM queue.jobs j WHERE j.id=a.job_id AND j.error_code='invalid_scope' AND a.finished_at IS NULL;
CREATE FUNCTION ingest.immutable_embedding_model() RETURNS trigger LANGUAGE plpgsql AS $$ BEGIN
 IF OLD.embedding_model_id IS NOT NULL AND NEW.embedding_model_id IS DISTINCT FROM OLD.embedding_model_id THEN
 RAISE EXCEPTION 'run embedding model is immutable'; END IF; RETURN NEW; END $$;
CREATE TRIGGER immutable_embedding_model BEFORE UPDATE ON ingest.runs FOR EACH ROW EXECUTE FUNCTION ingest.immutable_embedding_model();
GRANT USAGE ON SCHEMA queue TO screenroom_catalog,screenroom_worker;
GRANT SELECT,INSERT,UPDATE ON ALL TABLES IN SCHEMA queue TO screenroom_catalog,screenroom_worker;
