ALTER TABLE media.artwork ALTER COLUMN revision DROP NOT NULL;
ALTER TABLE media.artwork ADD COLUMN current_generation bigint NOT NULL DEFAULT 0 CHECK(current_generation>=0);
CREATE TABLE media.artwork_generations (
 artwork_id uuid NOT NULL, source_id uuid NOT NULL, revision bigint NOT NULL CHECK(revision>0),
 fingerprint bytea NOT NULL CHECK(octet_length(fingerprint)=32), manifest jsonb NOT NULL CHECK(jsonb_typeof(manifest)='object'),
 kind text NOT NULL, rights jsonb NOT NULL CHECK(jsonb_typeof(rights)='object'),
 state text NOT NULL DEFAULT 'pending' CHECK(state IN ('pending','published','superseded')),
 PRIMARY KEY(artwork_id,revision), FOREIGN KEY(artwork_id,source_id) REFERENCES media.artwork(id,source_id)
);
UPDATE media.artwork a SET current_generation=greatest(a.revision,coalesce((SELECT max(v.revision) FROM media.variants v WHERE v.artwork_id=a.id),0));
-- Legacy rows have no required-variant manifest; only complete original/native pairs are advertised.
INSERT INTO media.artwork_generations(artwork_id,source_id,revision,fingerprint,manifest,kind,rights,state)
 SELECT a.id,a.source_id,v.revision,sha256(convert_to('legacy:'||a.id::text||':'||v.revision::text,'UTF8')),
 jsonb_build_object('version',0),a.kind,a.rights,'published'
 FROM media.artwork a JOIN media.variants v ON v.artwork_id=a.id JOIN media.objects o ON o.id=v.object_id
 GROUP BY a.id,v.revision HAVING count(DISTINCT v.variant)=2 AND bool_and(o.state='ready');
UPDATE media.artwork a SET revision=(SELECT max(g.revision) FROM media.artwork_generations g WHERE g.artwork_id=a.id),
 state=CASE WHEN EXISTS(SELECT FROM media.artwork_generations g WHERE g.artwork_id=a.id) THEN a.state ELSE 'absent' END;
UPDATE ingest.jobs SET state='cancelled',lease_owner=NULL,lease_until=NULL
 WHERE kind='artwork' AND state IN ('ready','leased');
CREATE TABLE ingest.artwork_omissions (
 run_id uuid NOT NULL REFERENCES ingest.runs(id), source_artwork_id text NOT NULL CHECK(length(source_artwork_id) BETWEEN 1 AND 200),
 code text NOT NULL CHECK(code IN ('missing','invalid','unavailable')), observed_at timestamptz NOT NULL DEFAULT now(),
 PRIMARY KEY(run_id,source_artwork_id)
);
GRANT SELECT ON media.artwork_generations,ingest.artwork_omissions TO screenroom_catalog;
GRANT SELECT,INSERT,UPDATE,DELETE ON media.artwork_generations,ingest.artwork_omissions TO screenroom_worker;
