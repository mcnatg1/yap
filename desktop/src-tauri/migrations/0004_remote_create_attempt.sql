ALTER TABLE prepared_remote_jobs
ADD COLUMN create_attempt_base_url TEXT CHECK (
  create_attempt_base_url IS NULL
  OR length(create_attempt_base_url) BETWEEN 1 AND 2048
);

PRAGMA user_version = 4;
