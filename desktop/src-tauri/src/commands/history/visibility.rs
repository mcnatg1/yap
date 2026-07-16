use std::{collections::HashSet, path::Path, sync::Mutex, time::Duration};

use rusqlite::{params, Connection, TransactionBehavior};

use super::{HistoryOrigin, NativeHistoryIdentity, MAX_HISTORY_SESSIONS};

const CURRENT_SCHEMA_VERSION: i64 = 1;

enum VisibilityBackend {
    Available(Connection),
    Unavailable(String),
}

pub(super) struct HistoryVisibility {
    backend: Mutex<VisibilityBackend>,
}

impl HistoryVisibility {
    pub(super) fn open_default() -> Self {
        let path = crate::paths::app_data_dir().join("history-visibility.sqlite3");
        match Self::open(&path) {
            Ok(store) => store,
            Err(error) => Self {
                backend: Mutex::new(VisibilityBackend::Unavailable(error)),
            },
        }
    }

    fn open(path: &Path) -> Result<Self, String> {
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent).map_err(|error| {
                format!("history preference directory could not be created: {error}")
            })?;
        }
        let connection = Connection::open(path)
            .map_err(|error| format!("history preference database could not be opened: {error}"))?;
        connection
            .busy_timeout(Duration::from_secs(2))
            .map_err(|error| format!("history preference database timeout failed: {error}"))?;
        let schema_version = connection
            .query_row("PRAGMA user_version", [], |row| row.get::<_, i64>(0))
            .map_err(|error| format!("history preference schema could not be read: {error}"))?;
        if schema_version > CURRENT_SCHEMA_VERSION {
            return Err(format!(
                "history preference schema {schema_version} is newer than supported schema {CURRENT_SCHEMA_VERSION}"
            ));
        }
        connection
            .execute_batch(
                "PRAGMA synchronous = FULL;
                 CREATE TABLE IF NOT EXISTS hidden_native_history (
                    hidden_order INTEGER PRIMARY KEY AUTOINCREMENT,
                    origin TEXT NOT NULL CHECK (origin IN ('live', 'remote')),
                    session_id TEXT NOT NULL,
                    output_path TEXT NOT NULL,
                    UNIQUE (origin, session_id, output_path)
                 );
                 PRAGMA user_version = 1;",
            )
            .map_err(|error| {
                format!("history preference schema could not be initialized: {error}")
            })?;
        connection
            .prepare(
                "SELECT hidden_order, origin, session_id, output_path
                 FROM hidden_native_history
                 LIMIT 0",
            )
            .map_err(|error| format!("history preference schema is incompatible: {error}"))?;
        Ok(Self {
            backend: Mutex::new(VisibilityBackend::Available(connection)),
        })
    }

    pub(super) fn hidden_identities(&self) -> Result<HashSet<NativeHistoryIdentity>, String> {
        let mut backend = self
            .backend
            .lock()
            .map_err(|_| "history preference database lock is unavailable".to_owned())?;
        let connection = match &mut *backend {
            VisibilityBackend::Available(connection) => connection,
            VisibilityBackend::Unavailable(error) => return Err(error.clone()),
        };
        let mut statement = connection
            .prepare(
                "SELECT origin, session_id, output_path
                 FROM hidden_native_history
                 ORDER BY hidden_order DESC
                 LIMIT ?1",
            )
            .map_err(|error| format!("hidden history preferences could not be queried: {error}"))?;
        let rows = statement
            .query_map([MAX_HISTORY_SESSIONS as i64], |row| {
                Ok((
                    row.get::<_, String>(0)?,
                    row.get::<_, String>(1)?,
                    row.get::<_, String>(2)?,
                ))
            })
            .map_err(|error| format!("hidden history preferences could not be queried: {error}"))?;
        let mut hidden = HashSet::new();
        for row in rows {
            let (origin, session_id, output_path) = row
                .map_err(|error| format!("hidden history preference could not be read: {error}"))?;
            let origin = match origin.as_str() {
                "live" => HistoryOrigin::Live,
                "remote" => HistoryOrigin::Remote,
                value => {
                    return Err(format!(
                        "hidden history preference has unsupported origin {value}"
                    ))
                }
            };
            hidden.insert(NativeHistoryIdentity {
                origin,
                session_id,
                output_path,
            });
        }
        Ok(hidden)
    }

    /// Persists identities ordered newest first, matching the client history contract.
    pub(super) fn hide_many(&self, identities: &[NativeHistoryIdentity]) -> Result<(), String> {
        let mut backend = self
            .backend
            .lock()
            .map_err(|_| "history preference database lock is unavailable".to_owned())?;
        let connection = match &mut *backend {
            VisibilityBackend::Available(connection) => connection,
            VisibilityBackend::Unavailable(error) => return Err(error.clone()),
        };
        let transaction = connection
            .transaction_with_behavior(TransactionBehavior::Immediate)
            .map_err(|error| format!("hidden history preference transaction failed: {error}"))?;
        for identity in identities.iter().rev() {
            let origin = match identity.origin {
                HistoryOrigin::Live => "live",
                HistoryOrigin::Remote => "remote",
            };
            transaction
                .execute(
                    "DELETE FROM hidden_native_history
                     WHERE origin = ?1 AND session_id = ?2 AND output_path = ?3",
                    params![origin, identity.session_id, identity.output_path],
                )
                .map_err(|error| {
                    format!("hidden history preference could not be refreshed: {error}")
                })?;
            transaction
                .execute(
                    "INSERT INTO hidden_native_history (origin, session_id, output_path)
                     VALUES (?1, ?2, ?3)",
                    params![origin, identity.session_id, identity.output_path],
                )
                .map_err(|error| {
                    format!("hidden history preference could not be saved: {error}")
                })?;
        }
        transaction
            .execute(
                "DELETE FROM hidden_native_history
                 WHERE hidden_order NOT IN (
                    SELECT hidden_order
                    FROM hidden_native_history
                    ORDER BY hidden_order DESC
                    LIMIT ?1
                 )",
                [MAX_HISTORY_SESSIONS as i64],
            )
            .map_err(|error| format!("hidden history preferences could not be bounded: {error}"))?;
        transaction
            .commit()
            .map_err(|error| format!("hidden history preferences could not be committed: {error}"))
    }
}

#[cfg(test)]
mod tests {
    use std::{
        sync::atomic::{AtomicU64, Ordering},
        time::{SystemTime, UNIX_EPOCH},
    };

    use super::*;

    static NEXT_TEST_DIR: AtomicU64 = AtomicU64::new(0);

    fn test_database_path(name: &str) -> std::path::PathBuf {
        let nonce = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        std::env::temp_dir()
            .join(format!(
                "yap-history-visibility-{name}-{}-{nonce}-{}",
                std::process::id(),
                NEXT_TEST_DIR.fetch_add(1, Ordering::Relaxed)
            ))
            .join("history.sqlite3")
    }

    fn identity(index: usize) -> NativeHistoryIdentity {
        NativeHistoryIdentity {
            origin: HistoryOrigin::Remote,
            session_id: format!("remote-{index}"),
            output_path: format!("remote-{index}.txt"),
        }
    }

    #[test]
    fn native_visibility_is_bounded_and_survives_reopen() {
        let path = test_database_path("bounded");
        let identities = (0..=MAX_HISTORY_SESSIONS).map(identity).collect::<Vec<_>>();
        {
            let store = HistoryVisibility::open(&path).unwrap();
            store.hide_many(&identities).unwrap();
            let hidden = store.hidden_identities().unwrap();
            assert_eq!(hidden.len(), MAX_HISTORY_SESSIONS);
            assert!(hidden.contains(&identity(0)));
            assert!(!hidden.contains(&identity(MAX_HISTORY_SESSIONS)));
        }
        {
            let reopened = HistoryVisibility::open(&path).unwrap();
            let hidden = reopened.hidden_identities().unwrap();
            assert_eq!(hidden.len(), MAX_HISTORY_SESSIONS);
            assert!(hidden.contains(&identity(0)));
            assert!(!hidden.contains(&identity(MAX_HISTORY_SESSIONS)));
        }
        std::fs::remove_dir_all(path.parent().unwrap()).unwrap();
    }
}
