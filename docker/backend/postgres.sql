DO $$ BEGIN
IF NOT EXISTS (SELECT FROM pg_roles WHERE rolname='screenroom_user') THEN CREATE ROLE screenroom_user NOLOGIN; END IF;
IF NOT EXISTS (SELECT FROM pg_roles WHERE rolname='screenroom_catalog') THEN CREATE ROLE screenroom_catalog NOLOGIN; END IF;
IF NOT EXISTS (SELECT FROM pg_roles WHERE rolname='screenroom_worker') THEN CREATE ROLE screenroom_worker NOLOGIN; END IF;
IF NOT EXISTS (SELECT FROM pg_roles WHERE rolname='user_runtime') THEN CREATE ROLE user_runtime LOGIN INHERIT PASSWORD 'user-local-only' IN ROLE screenroom_user; END IF;
IF NOT EXISTS (SELECT FROM pg_roles WHERE rolname='catalog_runtime') THEN CREATE ROLE catalog_runtime LOGIN INHERIT PASSWORD 'catalog-local-only' IN ROLE screenroom_catalog; END IF;
IF NOT EXISTS (SELECT FROM pg_roles WHERE rolname='worker_runtime') THEN CREATE ROLE worker_runtime LOGIN INHERIT PASSWORD 'worker-local-only' IN ROLE screenroom_worker; END IF;
END $$;
