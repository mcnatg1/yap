PRAGMA foreign_keys = ON;

CREATE TABLE recording_jobs (
  job_id TEXT PRIMARY KEY,
  session_mode TEXT NOT NULL CHECK (session_mode IN ('dictation', 'meeting')),
  session_origin TEXT NOT NULL CHECK (session_origin IN ('live_capture', 'imported_file')),
  source_path TEXT,
  source_ownership TEXT NOT NULL DEFAULT 'external' CHECK (source_ownership IN ('external', 'yap_spool')),
  output_path TEXT,
  display_name TEXT NOT NULL,
  status TEXT NOT NULL CHECK (status IN (
    'accepted', 'preflighting', 'blocked_setup_required',
    'blocked_server_unavailable', 'blocked_sign_in_required',
    'queued_local_fallback', 'queued_server', 'preprocessing',
    'uploading', 'server_processing', 'local_transcribing', 'saving',
    'diarization_queued', 'diarization_running', 'complete', 'partial',
    'failed', 'cancelled'
  )),
  route TEXT CHECK (route IS NULL OR route IN ('local_fallback', 'server_batch', 'server_live')),
  attempt_count INTEGER NOT NULL DEFAULT 0 CHECK (attempt_count >= 0),
  next_attempt_at_ms INTEGER,
  cancellation_requested INTEGER NOT NULL DEFAULT 0 CHECK (cancellation_requested IN (0, 1)),
  capture_commit_path TEXT,
  capture_manifest_sha256 TEXT,
  error_code TEXT,
  error_message TEXT,
  created_at_ms INTEGER NOT NULL,
  updated_at_ms INTEGER NOT NULL,
  expires_at_ms INTEGER,
  CHECK (session_origin = 'live_capture' OR source_path IS NOT NULL)
);

CREATE INDEX recording_jobs_status_retry_idx
  ON recording_jobs(status, next_attempt_at_ms, created_at_ms);

CREATE TABLE job_chunks (
  job_id TEXT NOT NULL REFERENCES recording_jobs(job_id) ON DELETE CASCADE,
  owner_namespace TEXT NOT NULL,
  session_id TEXT NOT NULL,
  track_id TEXT NOT NULL,
  sequence_start INTEGER NOT NULL,
  sequence_end INTEGER NOT NULL,
  content_sha256 TEXT NOT NULL,
  artifact_path TEXT NOT NULL,
  upload_offset INTEGER NOT NULL DEFAULT 0,
  acknowledged_object_id TEXT,
  acknowledged_at_ms INTEGER,
  PRIMARY KEY (job_id, track_id, sequence_start, sequence_end),
  CHECK (sequence_end >= sequence_start),
  CHECK (upload_offset >= 0)
);

PRAGMA user_version = 1;
