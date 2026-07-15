ALTER TABLE job_chunks ADD COLUMN content_byte_length INTEGER NOT NULL DEFAULT 0
  CHECK (content_byte_length >= 0);

CREATE TABLE prepared_remote_jobs (
  job_id TEXT PRIMARY KEY REFERENCES recording_jobs(job_id) ON DELETE CASCADE,
  create_request_json TEXT NOT NULL CHECK (
    length(create_request_json) BETWEEN 2 AND 1048576
  ),
  capture_manifest_path TEXT NOT NULL,
  capture_manifest_sha256 TEXT NOT NULL CHECK (
    length(capture_manifest_sha256) = 64
    AND capture_manifest_sha256 NOT GLOB '*[^0-9a-f]*'
  ),
  server_job_id TEXT UNIQUE,
  server_base_url TEXT,
  server_cancellation_acknowledged_at_ms INTEGER CHECK (
    server_cancellation_acknowledged_at_ms IS NULL
    OR server_cancellation_acknowledged_at_ms >= 0
  ),
  CHECK (
    (server_job_id IS NULL AND server_base_url IS NULL)
    OR (server_job_id IS NOT NULL AND server_base_url IS NOT NULL)
  )
);

CREATE TABLE detached_remote_cancellations (
  server_base_url TEXT NOT NULL CHECK (
    length(server_base_url) BETWEEN 1 AND 2048
  ),
  server_job_id TEXT NOT NULL CHECK (
    length(server_job_id) BETWEEN 1 AND 128
    AND server_job_id NOT GLOB '*[^A-Za-z0-9_-]*'
  ),
  create_request_json TEXT NOT NULL CHECK (
    length(create_request_json) BETWEEN 2 AND 1048576
  ),
  queued_at_ms INTEGER NOT NULL CHECK (queued_at_ms >= 0),
  PRIMARY KEY (server_base_url, server_job_id)
);

PRAGMA user_version = 2;
