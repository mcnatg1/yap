CREATE TABLE remote_spool_cleanup (
  job_id TEXT PRIMARY KEY CHECK (
    length(job_id) BETWEEN 1 AND 128
    AND job_id NOT GLOB '*[^A-Za-z0-9_-]*'
  ),
  queued_at_ms INTEGER NOT NULL CHECK (queued_at_ms >= 0)
);

PRAGMA user_version = 3;
