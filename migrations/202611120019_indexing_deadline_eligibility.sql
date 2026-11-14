CREATE FUNCTION ingest.indexing_deadline_valid(run_id uuid) RETURNS boolean
LANGUAGE sql STABLE AS $$
 SELECT NOT EXISTS(
  SELECT FROM ingest.indexing_runs ir JOIN ingest.runs r ON r.id=ir.run_id
  WHERE r.id=$1 AND r.started_at+make_interval(secs=>(r.policy_snapshot->'selection_json'->'limits'->>'deadline_seconds')::integer)<=statement_timestamp()
 )
$$;
GRANT EXECUTE ON FUNCTION ingest.indexing_deadline_valid(uuid) TO screenroom_catalog,screenroom_worker;
