CREATE EXTENSION IF NOT EXISTS vector;
CREATE EXTENSION IF NOT EXISTS pg_trgm;
DO $$ BEGIN
  IF current_setting('server_version_num')::integer < 180000 THEN
    RAISE EXCEPTION 'Screenroom requires PostgreSQL 18 or newer';
  END IF;
  IF NOT EXISTS (SELECT FROM pg_roles WHERE rolname='screenroom_user') THEN CREATE ROLE screenroom_user NOLOGIN; END IF;
  IF NOT EXISTS (SELECT FROM pg_roles WHERE rolname='screenroom_catalog') THEN CREATE ROLE screenroom_catalog NOLOGIN; END IF;
  IF NOT EXISTS (SELECT FROM pg_roles WHERE rolname='screenroom_worker') THEN CREATE ROLE screenroom_worker NOLOGIN; END IF;
END $$;
CREATE SCHEMA identity;
CREATE SCHEMA catalog;
CREATE SCHEMA ingest;
CREATE SCHEMA search;
CREATE SCHEMA media;
REVOKE CREATE ON SCHEMA public FROM PUBLIC;

CREATE TABLE identity.users (
 id uuid PRIMARY KEY DEFAULT gen_random_uuid(), public_id text NOT NULL UNIQUE CHECK (length(public_id) BETWEEN 1 AND 200),
 display_name text NOT NULL, state text NOT NULL DEFAULT 'enabled' CHECK(state IN ('enabled','disabled')),
 created_at timestamptz NOT NULL DEFAULT now(), updated_at timestamptz NOT NULL DEFAULT now()
);
CREATE TABLE identity.jellyfin_user_links (
 user_id uuid NOT NULL REFERENCES identity.users(id), source_id uuid NOT NULL,
 jellyfin_user_id text NOT NULL CHECK(length(jellyfin_user_id) BETWEEN 1 AND 200), updated_at timestamptz NOT NULL DEFAULT now(),
 PRIMARY KEY(user_id,source_id)
);
CREATE TABLE catalog.sources (
 id uuid PRIMARY KEY DEFAULT gen_random_uuid(), source_key text NOT NULL UNIQUE, provider text NOT NULL,
 state text NOT NULL DEFAULT 'enabled' CHECK(state IN ('enabled','disabled','deleted')),
 configuration jsonb NOT NULL DEFAULT '{}', capabilities jsonb NOT NULL DEFAULT '{}'
);
CREATE TABLE catalog.libraries (
 id uuid PRIMARY KEY DEFAULT gen_random_uuid(), source_id uuid NOT NULL REFERENCES catalog.sources(id),
 public_id text NOT NULL, external_id text NOT NULL, title text NOT NULL,
 state text NOT NULL DEFAULT 'enabled' CHECK(state IN ('enabled','disabled','absent')),
 UNIQUE(source_id,external_id), UNIQUE(source_id,public_id), UNIQUE(id,source_id)
);
CREATE TABLE catalog.items (
 id uuid PRIMARY KEY DEFAULT gen_random_uuid(), public_id text NOT NULL UNIQUE,
 kind text NOT NULL CHECK(kind IN ('movie','series','season','episode')), parent_id uuid REFERENCES catalog.items(id),
 created_at timestamptz NOT NULL DEFAULT now(), updated_at timestamptz NOT NULL DEFAULT now()
);
CREATE TABLE catalog.source_items (
 id uuid PRIMARY KEY DEFAULT gen_random_uuid(), source_id uuid NOT NULL REFERENCES catalog.sources(id), item_id uuid NOT NULL REFERENCES catalog.items(id),
 external_id text NOT NULL, source_revision text, metadata jsonb NOT NULL, provenance jsonb NOT NULL DEFAULT '{}', content_hash bytea NOT NULL CHECK(octet_length(content_hash)=32),
 state text NOT NULL DEFAULT 'enabled' CHECK(state IN ('enabled','absent','deleted')), updated_at timestamptz NOT NULL DEFAULT now(),
 UNIQUE(source_id,external_id), UNIQUE(id,source_id)
);
CREATE INDEX source_items_item ON catalog.source_items(item_id);
CREATE TABLE catalog.library_items (
 library_id uuid NOT NULL, source_item_id uuid NOT NULL, source_id uuid NOT NULL,
 last_seen_run_id uuid NOT NULL, state text NOT NULL DEFAULT 'enabled' CHECK(state IN ('enabled','absent')),
 PRIMARY KEY(library_id,source_item_id), FOREIGN KEY(library_id,source_id) REFERENCES catalog.libraries(id,source_id),
 FOREIGN KEY(source_item_id,source_id) REFERENCES catalog.source_items(id,source_id)
);
CREATE INDEX library_items_source ON catalog.library_items(source_item_id);
CREATE TABLE catalog.external_ids (
 source_item_id uuid NOT NULL REFERENCES catalog.source_items(id), provider text NOT NULL, entity_kind text NOT NULL,
 external_id text NOT NULL, evidence jsonb NOT NULL DEFAULT '{}', PRIMARY KEY(source_item_id,provider,entity_kind,external_id)
);
CREATE INDEX external_ids_lookup ON catalog.external_ids(provider,entity_kind,external_id);
CREATE TABLE catalog.people (
 id uuid PRIMARY KEY DEFAULT gen_random_uuid(), source_id uuid NOT NULL REFERENCES catalog.sources(id), external_id text NOT NULL,
 name text NOT NULL, metadata jsonb NOT NULL DEFAULT '{}', UNIQUE(source_id,external_id), UNIQUE(id,source_id)
);
CREATE TABLE catalog.credits (
 source_item_id uuid NOT NULL, person_id uuid NOT NULL, source_id uuid NOT NULL, department text NOT NULL, role text NOT NULL,
 ordinal integer NOT NULL CHECK(ordinal>=0), PRIMARY KEY(source_item_id,person_id,department,role),
 FOREIGN KEY(source_item_id,source_id) REFERENCES catalog.source_items(id,source_id), FOREIGN KEY(person_id,source_id) REFERENCES catalog.people(id,source_id)
);
CREATE TABLE ingest.policies (
 id uuid PRIMARY KEY DEFAULT gen_random_uuid(), source_id uuid NOT NULL REFERENCES catalog.sources(id), version bigint NOT NULL DEFAULT 1 CHECK(version>0),
 enabled boolean NOT NULL DEFAULT true, selection_json jsonb NOT NULL, max_items bigint NOT NULL CHECK(max_items>0), max_requests bigint NOT NULL CHECK(max_requests>0),
 incremental_seconds integer NOT NULL CHECK(incremental_seconds>0), reconcile_seconds integer NOT NULL CHECK(reconcile_seconds>0),
 enrich_metadata boolean NOT NULL, enrich_people boolean NOT NULL, enrich_artwork boolean NOT NULL, next_due_at timestamptz NOT NULL DEFAULT now()
);
CREATE TABLE ingest.runs (
 id uuid PRIMARY KEY DEFAULT gen_random_uuid(), policy_id uuid NOT NULL REFERENCES ingest.policies(id), policy_version bigint NOT NULL,
 policy_snapshot jsonb NOT NULL, selection_snapshot jsonb NOT NULL,
 state text NOT NULL DEFAULT 'running' CHECK(state IN ('running','paused','complete','failed','cancelled')),
 items bigint NOT NULL DEFAULT 0 CHECK(items>=0), requests bigint NOT NULL DEFAULT 0 CHECK(requests>=0),
 started_at timestamptz NOT NULL DEFAULT now(), finished_at timestamptz, coverage jsonb NOT NULL DEFAULT '{}'
);
CREATE UNIQUE INDEX one_active_run ON ingest.runs(policy_id) WHERE state IN ('running','paused');
CREATE FUNCTION ingest.immutable_snapshot() RETURNS trigger LANGUAGE plpgsql AS $$ BEGIN
 IF NEW.policy_snapshot IS DISTINCT FROM OLD.policy_snapshot OR NEW.selection_snapshot IS DISTINCT FROM OLD.selection_snapshot OR NEW.policy_version<>OLD.policy_version OR NEW.policy_id<>OLD.policy_id THEN
 RAISE EXCEPTION 'run snapshot is immutable'; END IF; RETURN NEW; END $$;
CREATE TRIGGER immutable_run BEFORE UPDATE ON ingest.runs FOR EACH ROW EXECUTE FUNCTION ingest.immutable_snapshot();
CREATE TABLE ingest.jobs (
 id uuid PRIMARY KEY DEFAULT gen_random_uuid(), run_id uuid REFERENCES ingest.runs(id),
 kind text NOT NULL CHECK(kind IN ('ingest','embedding','artwork')), dedupe_key text NOT NULL,
 state text NOT NULL DEFAULT 'ready' CHECK(state IN ('ready','leased','complete','failed','cancelled')),
 available_at timestamptz NOT NULL DEFAULT now(), lease_owner text, lease_until timestamptz, fence bigint NOT NULL DEFAULT 0,
 attempts integer NOT NULL DEFAULT 0, max_attempts integer NOT NULL DEFAULT 5 CHECK(max_attempts BETWEEN 1 AND 20),
 input jsonb NOT NULL CHECK(octet_length(input::text)<=65536), error_code text,
 CHECK ((state='leased')=(lease_owner IS NOT NULL AND lease_until IS NOT NULL))
);
CREATE UNIQUE INDEX active_job_dedupe ON ingest.jobs(kind,dedupe_key) WHERE state IN ('ready','leased');
CREATE INDEX ready_jobs ON ingest.jobs(available_at,id) WHERE state IN ('ready','leased');
CREATE TABLE ingest.checkpoints (
 source_id uuid NOT NULL REFERENCES catalog.sources(id), scope_key text NOT NULL, policy_version bigint NOT NULL,
 cursor jsonb NOT NULL, watermark text, updated_at timestamptz NOT NULL DEFAULT now(), PRIMARY KEY(source_id,scope_key,policy_version)
);
CREATE TABLE ingest.feed_memberships (
 source_id uuid NOT NULL, feed_key text NOT NULL, source_item_id uuid NOT NULL, ordinal integer NOT NULL CHECK(ordinal>=0), last_seen_run_id uuid NOT NULL REFERENCES ingest.runs(id),
 PRIMARY KEY(source_id,feed_key,source_item_id), FOREIGN KEY(source_item_id,source_id) REFERENCES catalog.source_items(id,source_id)
);
CREATE TABLE search.documents (
 id uuid PRIMARY KEY DEFAULT gen_random_uuid(), source_item_id uuid NOT NULL UNIQUE REFERENCES catalog.source_items(id),
 document_version integer NOT NULL CHECK(document_version>0), content_hash bytea NOT NULL CHECK(octet_length(content_hash)=32),
 text text NOT NULL CHECK(length(text)<=32000), title text NOT NULL,
 lexical tsvector GENERATED ALWAYS AS (to_tsvector('english',text)) STORED, updated_at timestamptz NOT NULL DEFAULT now()
);
CREATE INDEX documents_lexical ON search.documents USING gin(lexical);
CREATE INDEX documents_title ON search.documents USING gin(title gin_trgm_ops);
CREATE TABLE search.embedding_models (
 id uuid PRIMARY KEY DEFAULT gen_random_uuid(), name text NOT NULL, revision text NOT NULL, dimensions integer NOT NULL CHECK(dimensions=1024),
 distance_metric text NOT NULL DEFAULT 'cosine' CHECK(distance_metric='cosine'), document_version integer NOT NULL,
 state text NOT NULL DEFAULT 'building' CHECK(state IN ('building','active','retired')), UNIQUE(name,revision)
);
CREATE UNIQUE INDEX one_active_model ON search.embedding_models(state) WHERE state='active';
CREATE TABLE search.embeddings (
 document_id uuid NOT NULL REFERENCES search.documents(id), model_id uuid NOT NULL REFERENCES search.embedding_models(id),
 input_hash bytea NOT NULL CHECK(octet_length(input_hash)=32), vector vector(1024) NOT NULL, created_at timestamptz NOT NULL DEFAULT now(), PRIMARY KEY(document_id,model_id)
);
CREATE TABLE search.result_snapshots (
 id uuid PRIMARY KEY DEFAULT gen_random_uuid(), digest bytea NOT NULL CHECK(octet_length(digest)=32),
 model_revision text, index_revision text NOT NULL, results jsonb NOT NULL CHECK(jsonb_array_length(results)<=1000),
 effective_methods jsonb NOT NULL, degradation_reason text NOT NULL DEFAULT '', expires_at timestamptz NOT NULL
);
CREATE INDEX snapshots_expiry ON search.result_snapshots(expires_at);
CREATE TABLE media.objects (
 id uuid PRIMARY KEY DEFAULT gen_random_uuid(), storage_scope uuid NOT NULL REFERENCES catalog.sources(id),
 sha256 bytea NOT NULL CHECK(octet_length(sha256)=32), bucket text NOT NULL, object_key text NOT NULL UNIQUE, mime text NOT NULL,
 byte_length bigint NOT NULL CHECK(byte_length BETWEEN 1 AND 20971520), width integer NOT NULL CHECK(width BETWEEN 1 AND 16384), height integer NOT NULL CHECK(height BETWEEN 1 AND 16384),
 state text NOT NULL DEFAULT 'ready' CHECK(state IN ('ready','absent')), created_at timestamptz NOT NULL DEFAULT now(), UNIQUE(storage_scope,sha256), UNIQUE(id,storage_scope)
);
CREATE TABLE media.artwork (
 id uuid PRIMARY KEY DEFAULT gen_random_uuid(), source_id uuid NOT NULL REFERENCES catalog.sources(id), source_artwork_id text NOT NULL,
 kind text NOT NULL, rights jsonb NOT NULL, revision bigint NOT NULL CHECK(revision>0), state text NOT NULL DEFAULT 'enabled' CHECK(state IN ('enabled','absent')),
 UNIQUE(source_id,source_artwork_id), UNIQUE(id,source_id)
);
CREATE TABLE media.variants (
 artwork_id uuid NOT NULL, source_id uuid NOT NULL, revision bigint NOT NULL, variant text NOT NULL CHECK(variant IN ('original','native')),
 object_id uuid NOT NULL, PRIMARY KEY(artwork_id,revision,variant), FOREIGN KEY(artwork_id,source_id) REFERENCES media.artwork(id,source_id), FOREIGN KEY(object_id,source_id) REFERENCES media.objects(id,storage_scope)
);
CREATE TABLE media.item_artwork (
 source_item_id uuid NOT NULL, artwork_id uuid NOT NULL, source_id uuid NOT NULL, role text NOT NULL, ordinal integer NOT NULL CHECK(ordinal>=0),
 PRIMARY KEY(source_item_id,artwork_id,role), FOREIGN KEY(source_item_id,source_id) REFERENCES catalog.source_items(id,source_id), FOREIGN KEY(artwork_id,source_id) REFERENCES media.artwork(id,source_id)
);
CREATE TABLE media.person_artwork (
 person_id uuid NOT NULL, artwork_id uuid NOT NULL, source_id uuid NOT NULL, role text NOT NULL,
 PRIMARY KEY(person_id,artwork_id,role), FOREIGN KEY(person_id,source_id) REFERENCES catalog.people(id,source_id), FOREIGN KEY(artwork_id,source_id) REFERENCES media.artwork(id,source_id)
);
