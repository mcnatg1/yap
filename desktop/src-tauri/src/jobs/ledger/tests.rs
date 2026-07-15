use super::records::validate_server_base_url;
use super::*;
use crate::jobs::model::{transition_policy, TransitionPolicy};
use crate::jobs::{
    NewPreparedRemoteJob, RecordingJobStatus, RecordingRoute, SessionMode, SessionOrigin,
    SourceOwnership,
};
use rusqlite::types::ValueRef;
use std::{
    fs,
    sync::{
        atomic::{AtomicU64, Ordering},
        Arc, Barrier,
    },
    thread,
};

static NEXT_TEMP: AtomicU64 = AtomicU64::new(0);

#[test]
fn persisted_unknown_enum_is_reported_as_corruption() {
    let ledger = JobLedger::open_in_memory().unwrap();
    ledger.insert_job(&imported_job("bad-enum")).unwrap();
    {
        let connection = ledger.connection.lock().unwrap();
        connection
            .execute_batch("PRAGMA ignore_check_constraints = ON;")
            .unwrap();
        connection
            .execute(
                "UPDATE recording_jobs SET status = 'invented_ui_state' WHERE job_id = 'bad-enum'",
                [],
            )
            .unwrap();
    }
    assert!(matches!(
        ledger.get_job("bad-enum"),
        Err(JobLedgerError::CorruptValue {
            field: "status",
            ..
        })
    ));
}

#[test]
fn durable_remote_origins_use_the_same_numeric_loopback_contract() {
    assert!(validate_server_base_url("http://127.0.0.1:18765").is_ok());
    assert!(validate_server_base_url("http://[::1]:18765").is_ok());
    assert!(validate_server_base_url("http://localhost:18765").is_err());
    assert!(validate_server_base_url("http://127.0.0.1:18765/alternate").is_err());
}

#[test]
fn restart_recovers_nonterminal_jobs_and_chunks() {
    let dir = temp_dir("restart");
    let path = dir.join("jobs.sqlite3");
    let source = dir.join("interview.wav");
    fs::write(&source, b"RIFF-restart-fixture").unwrap();
    let mut job = imported_job_at("restart-job", source.clone());
    job.status = RecordingJobStatus::QueuedServer;
    job.route = Some(RecordingRoute::ServerBatch);
    let chunk = chunk_at(dir.join("chunk-0.flac"));
    {
        let ledger = JobLedger::open(&path).unwrap();
        ledger.insert_job_with_chunks(&job, &[chunk]).unwrap();
    }

    let ledger = JobLedger::open(&path).unwrap();
    let recovered = ledger.list_recoverable_jobs().unwrap();
    assert_eq!(recovered.len(), 1);
    assert_eq!(recovered[0].job_id, "restart-job");
    assert_eq!(recovered[0].source_path.as_deref(), Some(source.as_path()));
    assert_eq!(ledger.list_chunks("restart-job").unwrap().len(), 1);
    drop(ledger);
    fs::remove_dir_all(dir).unwrap();
}

mod remote_state;

mod concurrency;

mod lifecycle_retention;

#[test]
fn restart_database_has_exact_metadata_surface_and_no_payload_content() {
    let dir = temp_dir("content-audit");
    let path = dir.join("jobs.sqlite3");
    let source = dir.join("source.wav");
    let output = dir.join("output.txt");
    let artifact = dir.join("chunk.flac");
    let wav_bytes = b"RIFF\x00\x01YAP_PRIVATE_WAV_BYTES";
    let transcript = "YAP_PRIVATE_TRANSCRIPT_SENTENCE";
    fs::write(&source, wav_bytes).unwrap();
    fs::write(&output, transcript).unwrap();
    fs::write(&artifact, b"encoded audio bytes").unwrap();
    let mut job = imported_job_at("audit-job", source);
    job.output_path = Some(output);
    {
        let ledger = JobLedger::open(&path).unwrap();
        ledger
            .insert_job_with_chunks(&job, &[chunk_at(artifact)])
            .unwrap();
    }

    let connection = rusqlite::Connection::open(&path).unwrap();
    let table_names: Vec<String> = {
        let mut statement = connection.prepare(
            "SELECT name FROM sqlite_schema WHERE type = 'table' AND name NOT LIKE 'sqlite_%' ORDER BY name"
        ).unwrap();
        statement
            .query_map([], |row| row.get(0))
            .unwrap()
            .collect::<Result<_, _>>()
            .unwrap()
    };
    assert_eq!(
        table_names,
        [
            "detached_remote_cancellations",
            "job_chunks",
            "prepared_remote_jobs",
            "recording_jobs",
            "remote_spool_cleanup",
        ]
    );
    let expected_columns = [
        (
            "detached_remote_cancellations",
            &[
                ("server_base_url", "TEXT"),
                ("server_job_id", "TEXT"),
                ("create_request_json", "TEXT"),
                ("queued_at_ms", "INTEGER"),
            ][..],
        ),
        (
            "job_chunks",
            &[
                ("job_id", "TEXT"),
                ("owner_namespace", "TEXT"),
                ("session_id", "TEXT"),
                ("track_id", "TEXT"),
                ("sequence_start", "INTEGER"),
                ("sequence_end", "INTEGER"),
                ("content_sha256", "TEXT"),
                ("artifact_path", "TEXT"),
                ("upload_offset", "INTEGER"),
                ("acknowledged_object_id", "TEXT"),
                ("acknowledged_at_ms", "INTEGER"),
                ("content_byte_length", "INTEGER"),
            ][..],
        ),
        (
            "prepared_remote_jobs",
            &[
                ("job_id", "TEXT"),
                ("create_request_json", "TEXT"),
                ("capture_manifest_path", "TEXT"),
                ("capture_manifest_sha256", "TEXT"),
                ("server_job_id", "TEXT"),
                ("server_base_url", "TEXT"),
                ("server_cancellation_acknowledged_at_ms", "INTEGER"),
                ("create_attempt_base_url", "TEXT"),
            ][..],
        ),
        (
            "recording_jobs",
            &[
                ("job_id", "TEXT"),
                ("session_mode", "TEXT"),
                ("session_origin", "TEXT"),
                ("source_path", "TEXT"),
                ("source_ownership", "TEXT"),
                ("output_path", "TEXT"),
                ("display_name", "TEXT"),
                ("status", "TEXT"),
                ("route", "TEXT"),
                ("attempt_count", "INTEGER"),
                ("next_attempt_at_ms", "INTEGER"),
                ("cancellation_requested", "INTEGER"),
                ("capture_commit_path", "TEXT"),
                ("capture_manifest_sha256", "TEXT"),
                ("error_code", "TEXT"),
                ("error_message", "TEXT"),
                ("created_at_ms", "INTEGER"),
                ("updated_at_ms", "INTEGER"),
                ("expires_at_ms", "INTEGER"),
            ][..],
        ),
        (
            "remote_spool_cleanup",
            &[("job_id", "TEXT"), ("queued_at_ms", "INTEGER")][..],
        ),
    ];
    for (table, expected) in expected_columns {
        let actual: Vec<(String, String)> = {
            let mut statement = connection
                .prepare(&format!("PRAGMA table_info(\"{table}\")"))
                .unwrap();
            statement
                .query_map([], |row| Ok((row.get(1)?, row.get(2)?)))
                .unwrap()
                .collect::<Result<_, _>>()
                .unwrap()
        };
        assert_eq!(
            actual,
            expected
                .iter()
                .map(|(name, kind)| ((*name).into(), (*kind).into()))
                .collect::<Vec<(String, String)>>(),
            "{table} added an unapproved payload, credential, or embedding storage surface"
        );

        let mut statement = connection
            .prepare(&format!("SELECT * FROM \"{table}\""))
            .unwrap();
        let column_count = statement.column_count();
        let mut rows = statement.query([]).unwrap();
        while let Some(row) = rows.next().unwrap() {
            for column in 0..column_count {
                match row.get_ref(column).unwrap() {
                    ValueRef::Text(value) | ValueRef::Blob(value) => {
                        assert!(!value
                            .windows(wav_bytes.len())
                            .any(|window| window == wav_bytes));
                        let text = String::from_utf8_lossy(value);
                        assert!(!text.contains(transcript));
                    }
                    ValueRef::Null | ValueRef::Integer(_) | ValueRef::Real(_) => {}
                }
            }
        }
    }
    drop(connection);
    fs::remove_dir_all(dir).unwrap();
}

#[test]
fn relative_paths_are_rejected_before_persistence() {
    let ledger = JobLedger::open_in_memory().unwrap();
    let mut job = imported_job("relative-path");
    job.source_path = Some("relative.wav".into());
    assert!(matches!(
        ledger.insert_job(&job),
        Err(JobLedgerError::InvalidPath { .. })
    ));
    assert!(ledger.get_job("relative-path").unwrap().is_none());
}

fn imported_job(id: &str) -> NewRecordingJob {
    imported_job_at(id, std::env::temp_dir().join(format!("{id}.wav")))
}

fn imported_job_at(id: &str, source_path: std::path::PathBuf) -> NewRecordingJob {
    NewRecordingJob {
        job_id: id.into(),
        session_mode: SessionMode::Meeting,
        session_origin: SessionOrigin::ImportedFile,
        source_path: Some(source_path),
        source_ownership: SourceOwnership::External,
        output_path: None,
        display_name: format!("{id}.wav"),
        status: RecordingJobStatus::Accepted,
        route: None,
        attempt_count: 0,
        next_attempt_at_ms: None,
        cancellation_requested: false,
        capture_commit_path: None,
        capture_manifest_sha256: None,
        error_code: None,
        error_message: None,
        created_at_ms: 100,
        updated_at_ms: 100,
        expires_at_ms: None,
    }
}

fn chunk_at(artifact_path: std::path::PathBuf) -> NewJobChunk {
    NewJobChunk {
        owner_namespace: "local:test-install".into(),
        session_id: "session-1".into(),
        track_id: "microphone".into(),
        sequence_start: 0,
        sequence_end: 9,
        content_sha256: "0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef".into(),
        content_byte_length: 20,
        artifact_path,
        upload_offset: 0,
        acknowledged_object_id: None,
        acknowledged_at_ms: None,
    }
}

fn temp_dir(label: &str) -> std::path::PathBuf {
    let id = NEXT_TEMP.fetch_add(1, Ordering::Relaxed);
    let dir = std::env::temp_dir().join(format!("yap-ledger-{label}-{}-{id}", std::process::id()));
    fs::create_dir_all(&dir).unwrap();
    dir
}
