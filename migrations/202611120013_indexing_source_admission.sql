CREATE FUNCTION ingest.indexing_source_available(run_id uuid) RETURNS boolean
LANGUAGE sql STABLE AS $$
 SELECT NOT EXISTS(SELECT FROM ingest.indexing_runs ir WHERE ir.run_id=$1)
 OR NOT EXISTS(
  SELECT FROM queue.jobs j JOIN ingest.runs busy ON busy.id=j.scope_id
  JOIN ingest.runs current ON current.source_id=busy.source_id
  WHERE current.id=$1 AND j.state='leased' AND j.lease_until>statement_timestamp()
 )
$$;
GRANT EXECUTE ON FUNCTION ingest.indexing_source_available(uuid) TO screenroom_catalog,screenroom_worker;
ALTER TABLE ingest.request_receipts DROP CONSTRAINT request_receipts_outcome_check;
ALTER TABLE ingest.request_receipts ADD CONSTRAINT request_receipts_outcome_check
 CHECK(outcome IN ('unknown','success','retryable','permanent','unresolved','skipped'));
