UPDATE queue.jobs j
SET requirements=array_append(j.requirements,
 CASE WHEN ir.source_configuration->>'locator_revision' ~ '^[0-9a-f]{64}$'
 THEN 'locator:'||(ir.source_configuration->>'locator_revision')
 ELSE 'locator:unenrolled' END)
FROM ingest.indexing_runs ir
WHERE ir.run_id=j.scope_id AND j.queue_name='indexing'
AND NOT EXISTS(SELECT FROM unnest(j.requirements) requirement WHERE requirement LIKE 'locator:%');
