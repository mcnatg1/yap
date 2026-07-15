use crate::jobs::model::JobLedgerError;
use rusqlite::{Connection, OpenFlags, TransactionBehavior};
use std::{path::Path, time::Duration};

const CURRENT_SCHEMA_VERSION: i64 = 3;
const MIGRATION_1_SQL: &str = include_str!("../../migrations/0001_job_ledger.sql");
const MIGRATION_2_SQL: &str = include_str!("../../migrations/0002_prepared_remote_jobs.sql");
const MIGRATION_3_SQL: &str = include_str!("../../migrations/0003_remote_spool_cleanup.sql");

pub(super) fn open_file(path: &Path) -> Result<Connection, JobLedgerError> {
    open_file_with_migration_hook(path, || {})
}

fn open_file_with_migration_hook(
    path: &Path,
    before_migration_transaction: impl FnOnce(),
) -> Result<Connection, JobLedgerError> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    let mut connection = Connection::open_with_flags(
        path,
        OpenFlags::SQLITE_OPEN_READ_WRITE
            | OpenFlags::SQLITE_OPEN_CREATE
            | OpenFlags::SQLITE_OPEN_NO_MUTEX,
    )?;
    configure_connection(&connection, true)?;
    migrate_with_hook(&mut connection, before_migration_transaction)?;
    Ok(connection)
}

#[cfg(test)]
pub(super) fn open_in_memory() -> Result<Connection, JobLedgerError> {
    let mut connection = Connection::open_in_memory()?;
    configure_connection(&connection, false)?;
    migrate(&mut connection)?;
    Ok(connection)
}

#[cfg(test)]
fn migrate(connection: &mut Connection) -> Result<(), JobLedgerError> {
    migrate_with_hook(connection, || {})
}

fn migrate_with_hook(
    connection: &mut Connection,
    before_migration_transaction: impl FnOnce(),
) -> Result<(), JobLedgerError> {
    before_migration_transaction();
    let transaction = connection.transaction_with_behavior(TransactionBehavior::Immediate)?;
    let version: i64 = transaction.query_row("PRAGMA user_version", [], |row| row.get(0))?;
    match version {
        CURRENT_SCHEMA_VERSION => {}
        2 => transaction.execute_batch(MIGRATION_3_SQL)?,
        1 => {
            transaction.execute_batch(MIGRATION_2_SQL)?;
            transaction.execute_batch(MIGRATION_3_SQL)?;
        }
        0 => {
            transaction.execute_batch(MIGRATION_1_SQL)?;
            transaction.execute_batch(MIGRATION_2_SQL)?;
            transaction.execute_batch(MIGRATION_3_SQL)?;
        }
        unsupported => return Err(JobLedgerError::UnsupportedSchema(unsupported)),
    }
    transaction.commit()?;
    Ok(())
}

fn configure_connection(connection: &Connection, file_backed: bool) -> Result<(), JobLedgerError> {
    connection.busy_timeout(Duration::from_secs(5))?;
    connection.pragma_update(None, "foreign_keys", true)?;
    if file_backed {
        let journal_mode: String =
            connection.query_row("PRAGMA journal_mode = WAL", [], |row| row.get(0))?;
        if !journal_mode.eq_ignore_ascii_case("wal") {
            return Err(JobLedgerError::PragmaNotApplied {
                pragma: "journal_mode",
                requested: "WAL",
                actual: journal_mode,
            });
        }
    }
    connection.pragma_update(None, "synchronous", "FULL")?;
    Ok(())
}

#[cfg(test)]
fn migrate_with_sql(connection: &mut Connection, sql: &str) -> Result<(), JobLedgerError> {
    let transaction = connection.transaction_with_behavior(TransactionBehavior::Immediate)?;
    transaction.execute_batch(sql)?;
    transaction.commit()?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use rusqlite::Connection;
    use std::{
        fs,
        sync::atomic::{AtomicU64, Ordering},
        time::Duration,
    };

    static NEXT_TEMP: AtomicU64 = AtomicU64::new(0);

    #[test]
    fn migration_creates_versioned_constrained_schema_and_foreign_keys() {
        let connection = open_in_memory().unwrap();
        let version: i64 = connection
            .query_row("PRAGMA user_version", [], |row| row.get(0))
            .unwrap();
        let foreign_keys: i64 = connection
            .query_row("PRAGMA foreign_keys", [], |row| row.get(0))
            .unwrap();
        let tables: Vec<String> = {
            let mut statement = connection
                .prepare("SELECT name FROM sqlite_schema WHERE type = 'table' ORDER BY name")
                .unwrap();
            statement
                .query_map([], |row| row.get(0))
                .unwrap()
                .collect::<Result<_, _>>()
                .unwrap()
        };

        assert_eq!(version, 3);
        assert_eq!(foreign_keys, 1);
        assert_eq!(
            tables,
            [
                "detached_remote_cancellations",
                "job_chunks",
                "prepared_remote_jobs",
                "recording_jobs",
                "remote_spool_cleanup",
            ]
        );
        assert!(connection.execute(
            "INSERT INTO job_chunks (job_id, owner_namespace, session_id, track_id, sequence_start, sequence_end, content_sha256, artifact_path) VALUES ('missing', 'local:test', 'session', 'mic', 0, 1, 'hash', 'artifact')",
            [],
        ).is_err());
    }

    #[test]
    fn file_database_uses_wal_full_sync_and_five_second_timeout() {
        let dir = temp_dir("pragmas");
        let path = dir.join("jobs.sqlite3");
        let connection = open_file(&path).unwrap();
        let journal: String = connection
            .query_row("PRAGMA journal_mode", [], |row| row.get(0))
            .unwrap();
        let synchronous: i64 = connection
            .query_row("PRAGMA synchronous", [], |row| row.get(0))
            .unwrap();
        let busy_timeout: i64 = connection
            .query_row("PRAGMA busy_timeout", [], |row| row.get(0))
            .unwrap();

        assert_eq!(journal, "wal");
        assert_eq!(synchronous, 2);
        assert_eq!(busy_timeout, Duration::from_secs(5).as_millis() as i64);
        drop(connection);
        fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn file_configuration_rejects_a_journal_mode_other_than_wal() {
        let connection = Connection::open_in_memory().unwrap();

        let error = configure_connection(&connection, true).unwrap_err();

        assert!(error.to_string().contains("requested WAL"));
        assert!(error.to_string().contains("memory"));
    }

    #[test]
    fn failed_migration_rolls_back_every_schema_change() {
        let mut connection = Connection::open_in_memory().unwrap();
        configure_connection(&connection, false).unwrap();
        let error = migrate_with_sql(
            &mut connection,
            "CREATE TABLE should_rollback (id INTEGER); THIS IS NOT SQL; PRAGMA user_version = 1;",
        )
        .unwrap_err();

        assert!(error.to_string().contains("syntax"));
        let count: i64 = connection
            .query_row(
                "SELECT COUNT(*) FROM sqlite_schema WHERE name = 'should_rollback'",
                [],
                |row| row.get(0),
            )
            .unwrap();
        let version: i64 = connection
            .query_row("PRAGMA user_version", [], |row| row.get(0))
            .unwrap();
        assert_eq!(count, 0);
        assert_eq!(version, 0);
    }

    #[test]
    fn reopening_an_initialized_database_is_idempotent() {
        let dir = temp_dir("reopen");
        let path = dir.join("jobs.sqlite3");
        drop(open_file(&path).unwrap());
        let connection = open_file(&path).unwrap();
        let version: i64 = connection
            .query_row("PRAGMA user_version", [], |row| row.get(0))
            .unwrap();
        let table_count: i64 = connection.query_row(
            "SELECT COUNT(*) FROM sqlite_schema WHERE type = 'table' AND name IN ('recording_jobs', 'job_chunks', 'prepared_remote_jobs', 'detached_remote_cancellations', 'remote_spool_cleanup')",
            [],
            |row| row.get(0),
        ).unwrap();
        assert_eq!((version, table_count), (3, 5));
        drop(connection);
        fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn version_one_database_upgrades_without_replacing_existing_jobs() {
        let mut connection = Connection::open_in_memory().unwrap();
        configure_connection(&connection, false).unwrap();
        connection.execute_batch(MIGRATION_1_SQL).unwrap();
        connection.execute(
            "INSERT INTO recording_jobs (job_id, session_mode, session_origin, source_path, display_name, status, created_at_ms, updated_at_ms) VALUES ('existing', 'meeting', 'imported_file', 'C:/existing.wav', 'existing.wav', 'queued_server', 1, 1)",
            [],
        ).unwrap();

        migrate(&mut connection).unwrap();

        let version: i64 = connection
            .query_row("PRAGMA user_version", [], |row| row.get(0))
            .unwrap();
        let existing: String = connection
            .query_row(
                "SELECT display_name FROM recording_jobs WHERE job_id = 'existing'",
                [],
                |row| row.get(0),
            )
            .unwrap();
        let remote_table: i64 = connection
            .query_row(
                "SELECT COUNT(*) FROM sqlite_schema WHERE type = 'table' AND name = 'prepared_remote_jobs'",
                [],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(
            (version, existing.as_str(), remote_table),
            (3, "existing.wav", 1)
        );
    }

    #[test]
    fn concurrent_first_openers_share_one_atomic_migration_decision() {
        let dir = temp_dir("concurrent-first-open");
        let path = dir.join("jobs.sqlite3");
        let bootstrap = Connection::open(&path).unwrap();
        configure_connection(&bootstrap, true).unwrap();
        drop(bootstrap);
        let barrier = std::sync::Arc::new(std::sync::Barrier::new(2));
        let openers: Vec<_> = (0..2)
            .map(|_| {
                let path = path.clone();
                let barrier = std::sync::Arc::clone(&barrier);
                std::thread::spawn(move || {
                    open_file_with_migration_hook(&path, || {
                        barrier.wait();
                    })
                })
            })
            .collect();

        let connections: Vec<_> = openers
            .into_iter()
            .map(|opener| opener.join().unwrap())
            .collect();
        assert!(
            connections.iter().all(Result::is_ok),
            "both first openers must observe one idempotent migration: {connections:?}"
        );
        drop(connections);
        fs::remove_dir_all(dir).unwrap();
    }

    fn temp_dir(label: &str) -> std::path::PathBuf {
        let id = NEXT_TEMP.fetch_add(1, Ordering::Relaxed);
        let dir = std::env::temp_dir().join(format!(
            "yap-job-ledger-{label}-{}-{id}",
            std::process::id()
        ));
        fs::create_dir_all(&dir).unwrap();
        dir
    }
}
