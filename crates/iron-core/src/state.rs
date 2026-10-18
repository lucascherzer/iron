//! Bounded, portable application-state database.
//!
//! The identity key remains a separate private file. Keep the database's
//! containing directory private and writable: Turso may create WAL siblings.

use crate::platform::PrivateFileWriter;
use anyhow::{Context, Result, bail};
use iroh::EndpointId;
use std::path::Path;
use std::sync::Arc;
use tokio::sync::Mutex;
use turso::{Builder, Connection};

const VERSION: i64 = 1;

#[cfg(test)]
mod checkpoint_tests;

fn validate_checkpoint_status(status: (Option<i64>, Option<i64>, Option<i64>)) -> Result<()> {
    match status {
        (None, None, None) => Ok(()),
        (Some(busy), Some(log_frames), Some(checkpointed_frames))
            if busy == 0
                && ((log_frames == -1 && checkpointed_frames == -1)
                    || (log_frames >= 0
                        && checkpointed_frames >= 0
                        && checkpointed_frames >= log_frames)) =>
        {
            Ok(())
        }
        (Some(busy), Some(log_frames), Some(checkpointed_frames)) => {
            bail!(
                "incomplete WAL checkpoint (busy={busy}, log={log_frames}, checkpointed={checkpointed_frames})"
            )
        }
        _ => bail!("incomplete WAL checkpoint returned a partially NULL status"),
    }
}

/// Shared application-state store. One mutex owns the connection, so operations
/// are serialized and dropped transactions are cleaned up before its next use.
/// No database engine or connection escapes this domain API.
#[derive(Clone)]
pub struct AppState {
    connection: Arc<Mutex<Connection>>,
}

impl AppState {
    /// Opens a database, privately creating only a missing empty file.
    pub async fn open(path: &Path, writer: &dyn PrivateFileWriter) -> Result<Self> {
        if !path.exists() {
            writer.create_new(path, &[]).with_context(|| {
                format!("creating private application database {}", path.display())
            })?;
        }
        let db = Builder::new_local(path.to_str().context("database path is not UTF-8")?)
            .build()
            .await
            .with_context(|| format!("opening application database {}", path.display()))?;
        let mut conn = db.connect().context("connecting to application database")?;
        initialize(&mut conn, path).await?;
        Ok(Self {
            connection: Arc::new(Mutex::new(conn)),
        })
    }

    /// Loads the current peer snapshot.
    pub async fn load_peers(&self) -> Result<Vec<EndpointId>> {
        let conn = self.connection.lock().await;
        let mut rows = conn
            .query("SELECT endpoint_id FROM peers ORDER BY endpoint_id", ())
            .await
            .context("reading peers")?;
        let mut peers = Vec::new();
        while let Some(row) = rows.next().await? {
            let bytes: Vec<u8> = row.get(0)?;
            let bytes: [u8; 32] = bytes
                .try_into()
                .map_err(|_| anyhow::anyhow!("invalid EndpointId length in state database"))?;
            peers.push(
                EndpointId::from_bytes(&bytes).context("invalid EndpointId in state database")?,
            );
        }
        Ok(peers)
    }

    /// Checkpoints committed WAL contents into the main database file.
    ///
    /// This is useful before an offline, create-only migration publishes the
    /// database file elsewhere. All cloned stores must be dropped afterward.
    pub async fn checkpoint(&self) -> Result<()> {
        let conn = self.connection.lock().await;
        let mut rows = conn
            .query("PRAGMA wal_checkpoint(TRUNCATE)", ())
            .await
            .context("checkpointing state database")?;
        let row = rows
            .next()
            .await?
            .context("checkpoint returned no status")?;
        let busy: Option<i64> = row.get(0)?;
        let log_frames: Option<i64> = row.get(1)?;
        let checkpointed_frames: Option<i64> = row.get(2)?;
        validate_checkpoint_status((busy, log_frames, checkpointed_frames))
            .context("checkpointing state database")
    }

    /// Replaces the peer table with a snapshot atomically.
    pub async fn save_peers(&self, peers: impl IntoIterator<Item = EndpointId>) -> Result<()> {
        let mut conn = self.connection.lock().await;
        let transaction = conn.transaction().await?;
        transaction.execute("DELETE FROM peers", ()).await?;
        for id in peers {
            transaction
                .execute(
                    "INSERT INTO peers(endpoint_id) VALUES (?1)",
                    [id.as_bytes().to_vec()],
                )
                .await
                .context("saving peers")?;
        }
        transaction.commit().await?;
        Ok(())
    }

    /// Imports the legacy JSON array only when the database peer table is empty.
    /// Invalid IDs reject the complete import; nothing is partially committed.
    pub async fn import_legacy_peers(&self, bytes: &[u8]) -> Result<usize> {
        let peers = Self::decode_legacy_peers(bytes)?;
        let mut conn = self.connection.lock().await;
        let transaction = conn.transaction().await?;
        let mut rows = transaction.query("SELECT count(*) FROM peers", ()).await?;
        let count: i64 = rows.next().await?.context("missing peer count")?.get(0)?;
        drop(rows);
        if count != 0 {
            bail!("refusing legacy import into nonempty state database");
        }
        for id in &peers {
            transaction
                .execute(
                    "INSERT INTO peers(endpoint_id) VALUES (?1)",
                    [id.as_bytes().to_vec()],
                )
                .await
                .context("importing legacy peers")?;
        }
        transaction.commit().await?;
        Ok(peers.len())
    }

    /// Strictly decodes the legacy JSON array of base32 EndpointIds.
    /// Repeated valid IDs are ignored, preserving their first occurrence.
    pub fn decode_legacy_peers(bytes: &[u8]) -> Result<Vec<EndpointId>> {
        let strings: Vec<String> =
            serde_json::from_slice(bytes).context("parsing legacy peers JSON")?;
        let mut peers = strings
            .iter()
            .map(|value| {
                crate::id::parse_base32(value)
                    .with_context(|| format!("invalid EndpointId in legacy peers: {value}"))
            })
            .collect::<Result<Vec<_>>>()?;
        let mut seen = std::collections::HashSet::new();
        peers.retain(|id| seen.insert(*id));
        Ok(peers)
    }
}

async fn initialize(conn: &mut Connection, path: &Path) -> Result<()> {
    let mut rows = conn
        .query(
            "SELECT name FROM sqlite_master WHERE type='table' AND name='schema_version'",
            (),
        )
        .await?;
    let has_version = rows.next().await?.is_some();
    drop(rows);
    if !has_version {
        let mut tables = conn
            .query(
                "SELECT name FROM sqlite_master WHERE type='table' AND name NOT LIKE 'sqlite_%'",
                (),
            )
            .await?;
        let existing_table = tables.next().await?.is_some();
        drop(tables);
        if existing_table {
            bail!(
                "incompatible existing database schema at {}",
                path.display()
            );
        }
        let transaction = conn.transaction().await?;
        transaction
            .execute("CREATE TABLE schema_version (version INTEGER NOT NULL)", ())
            .await?;
        transaction
            .execute(
                "CREATE TABLE peers (endpoint_id BLOB PRIMARY KEY NOT NULL)",
                (),
            )
            .await?;
        transaction
            .execute("INSERT INTO schema_version(version) VALUES (?1)", [VERSION])
            .await?;
        transaction
            .commit()
            .await
            .with_context(|| format!("initializing application database {}", path.display()))?;
    } else {
        validate_table_shape(
            conn,
            "schema_version",
            &[("version", "INTEGER", true, false)],
            path,
        )
        .await?;
        validate_table_shape(conn, "peers", &[("endpoint_id", "BLOB", true, true)], path).await?;
        let mut rows = conn.query("SELECT version FROM schema_version", ()).await?;
        let row = rows.next().await?.context("empty schema version table")?;
        let version: i64 = row.get(0)?;
        let another_version = rows.next().await?.is_some();
        drop(rows);
        if version != VERSION {
            bail!("unsupported schema version {version} at {}", path.display());
        }
        if another_version {
            bail!(
                "schema version table must contain exactly one value at {}",
                path.display()
            );
        }
        let mut tables = conn.query("SELECT name FROM sqlite_master WHERE type='table' AND name NOT LIKE 'sqlite_%' AND name NOT IN ('schema_version','peers')", ()).await?;
        let extra_table = tables.next().await?.is_some();
        drop(tables);
        if extra_table {
            bail!(
                "incompatible extra tables in application database {}",
                path.display()
            );
        }
    }
    Ok(())
}

async fn validate_table_shape(
    conn: &Connection,
    table: &str,
    expected: &[(&str, &str, bool, bool)],
    path: &Path,
) -> Result<()> {
    let mut rows = conn
        .query(&format!("PRAGMA table_info({table})"), ())
        .await
        .with_context(|| format!("reading {table} schema at {}", path.display()))?;
    for (index, (name, declared_type, not_null, primary_key)) in expected.iter().enumerate() {
        let row = rows
            .next()
            .await
            .with_context(|| format!("reading {table} schema at {}", path.display()))?
            .with_context(|| format!("missing column {name} in {table} at {}", path.display()))?;
        let actual_name: String = row.get(1)?;
        let actual_type: String = row.get(2)?;
        let actual_not_null: i64 = row.get(3)?;
        let actual_primary_key: i64 = row.get(5)?;
        if actual_name != *name
            || !actual_type.eq_ignore_ascii_case(declared_type)
            || (actual_not_null != 0) != *not_null
            || (actual_primary_key != 0) != *primary_key
        {
            bail!(
                "incompatible {table} column {} at {} (expected {name} {declared_type}, NOT NULL={not_null}, PRIMARY KEY={primary_key}; found {actual_name} {actual_type}, NOT NULL={}, PRIMARY KEY={})",
                index + 1,
                path.display(),
                actual_not_null != 0,
                actual_primary_key != 0
            );
        }
    }
    if rows.next().await?.is_some() {
        bail!("unexpected extra columns in {table} at {}", path.display());
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_utils::{TestPrivateFileWriter, test_endpoint_id};

    #[tokio::test]
    async fn initializes_roundtrips_and_reopens_snapshot() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("state.db");
        let state = AppState::open(&path, &TestPrivateFileWriter).await.unwrap();
        let ids = [test_endpoint_id(1), test_endpoint_id(2)];
        state.save_peers(ids).await.unwrap();
        let mut expected = ids.to_vec();
        expected.sort_by_key(|id| id.as_bytes().to_vec());
        assert_eq!(state.load_peers().await.unwrap(), expected);
        assert!(state.save_peers([ids[0], ids[0]]).await.is_err());
        assert_eq!(state.load_peers().await.unwrap(), expected);
        state.checkpoint().await.unwrap();
        drop(state);
        let reopened = AppState::open(&path, &TestPrivateFileWriter).await.unwrap();
        assert_eq!(reopened.load_peers().await.unwrap(), expected);
        reopened.save_peers([]).await.unwrap();
        assert!(reopened.load_peers().await.unwrap().is_empty());
    }

    #[tokio::test]
    async fn cancelled_transaction_rolls_back_and_releases_the_connection() {
        use std::time::Duration;
        use tokio::time::timeout;

        let dir = tempfile::tempdir().unwrap();
        let state = AppState::open(&dir.path().join("state.db"), &TestPrivateFileWriter)
            .await
            .unwrap();
        let original = test_endpoint_id(10);
        state.save_peers([original]).await.unwrap();
        for write_before_cancel in [false, true] {
            let cancelled = state.clone();
            let (ready, reached_boundary) = tokio::sync::oneshot::channel();
            // Use the same mutex-owned connection and native transaction
            // lifetime as production. The explicit pause makes cancellation
            // after BEGIN/after a write deterministic without production hooks.
            let operation = tokio::spawn(async move {
                let mut conn = cancelled.connection.lock().await;
                let transaction = conn.transaction().await.unwrap();
                if write_before_cancel {
                    transaction.execute("DELETE FROM peers", ()).await.unwrap();
                    transaction
                        .execute(
                            "INSERT INTO peers(endpoint_id) VALUES (?1)",
                            [test_endpoint_id(11).as_bytes().to_vec()],
                        )
                        .await
                        .unwrap();
                }
                ready.send(()).unwrap();
                std::future::pending::<()>().await;
                transaction.commit().await.unwrap();
            });
            timeout(Duration::from_secs(5), reached_boundary)
                .await
                .unwrap()
                .unwrap();
            operation.abort();
            assert!(
                timeout(Duration::from_secs(5), operation)
                    .await
                    .unwrap()
                    .unwrap_err()
                    .is_cancelled()
            );
            let peers = timeout(Duration::from_secs(5), state.load_peers())
                .await
                .unwrap()
                .unwrap();
            assert_eq!(peers, vec![original]);
        }
        let next = test_endpoint_id(12);
        timeout(Duration::from_secs(5), state.save_peers([next]))
            .await
            .unwrap()
            .unwrap();
        assert_eq!(state.load_peers().await.unwrap(), vec![next]);
    }

    #[tokio::test]
    async fn imports_legacy_json_transactionally_and_refuses_clobber() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("state.db");
        let state = AppState::open(&path, &TestPrivateFileWriter).await.unwrap();
        let first = test_endpoint_id(3);
        let second = test_endpoint_id(4);
        let legacy = serde_json::to_vec(&vec![
            crate::id::to_base32(&first),
            crate::id::to_base32(&first),
            crate::id::to_base32(&second),
            crate::id::to_base32(&second),
        ])
        .unwrap();
        assert_eq!(
            AppState::decode_legacy_peers(&legacy).unwrap(),
            vec![first, second]
        );
        assert_eq!(state.import_legacy_peers(&legacy).await.unwrap(), 2);
        assert!(state.import_legacy_peers(&legacy).await.is_err());
        let mut expected = vec![first, second];
        expected.sort_by_key(|id| id.as_bytes().to_vec());
        assert_eq!(state.load_peers().await.unwrap(), expected);
    }

    #[tokio::test]
    async fn rejects_corrupt_unknown_and_incompatible_databases_without_reset() {
        let dir = tempfile::tempdir().unwrap();
        let corrupt = dir.path().join("corrupt.db");
        std::fs::write(&corrupt, b"not a database").unwrap();
        assert!(
            AppState::open(&corrupt, &TestPrivateFileWriter)
                .await
                .is_err()
        );
        assert_eq!(std::fs::read(&corrupt).unwrap(), b"not a database");

        let unknown = dir.path().join("unknown.db");
        let state = AppState::open(&unknown, &TestPrivateFileWriter)
            .await
            .unwrap();
        drop(state);
        let db = Builder::new_local(unknown.to_str().unwrap())
            .build()
            .await
            .unwrap();
        let conn = db.connect().unwrap();
        conn.execute("UPDATE schema_version SET version=42", ())
            .await
            .unwrap();
        drop(conn);
        drop(db);
        assert!(
            AppState::open(&unknown, &TestPrivateFileWriter)
                .await
                .is_err()
        );

        let incompatible = dir.path().join("other.db");
        let db = Builder::new_local(incompatible.to_str().unwrap())
            .build()
            .await
            .unwrap();
        db.connect()
            .unwrap()
            .execute("CREATE TABLE unrelated(value TEXT)", ())
            .await
            .unwrap();
        drop(db);
        assert!(
            AppState::open(&incompatible, &TestPrivateFileWriter)
                .await
                .is_err()
        );
    }

    #[tokio::test]
    async fn rejects_incompatible_version_one_table_shapes_without_mutation() {
        let cases = [
            (
                "peer_type",
                "CREATE TABLE peers (endpoint_id TEXT NOT NULL PRIMARY KEY)",
                "CREATE TABLE schema_version (version INTEGER NOT NULL)",
            ),
            (
                "peer_missing_primary_key",
                "CREATE TABLE peers (endpoint_id BLOB NOT NULL)",
                "CREATE TABLE schema_version (version INTEGER NOT NULL)",
            ),
            (
                "peer_nullable",
                "CREATE TABLE peers (endpoint_id BLOB PRIMARY KEY)",
                "CREATE TABLE schema_version (version INTEGER NOT NULL)",
            ),
            (
                "peer_missing_column",
                "CREATE TABLE peers (other BLOB NOT NULL PRIMARY KEY)",
                "CREATE TABLE schema_version (version INTEGER NOT NULL)",
            ),
            (
                "peer_extra_column",
                "CREATE TABLE peers (endpoint_id BLOB NOT NULL PRIMARY KEY, extra TEXT)",
                "CREATE TABLE schema_version (version INTEGER NOT NULL)",
            ),
            (
                "version_type",
                "CREATE TABLE peers (endpoint_id BLOB NOT NULL PRIMARY KEY)",
                "CREATE TABLE schema_version (version TEXT NOT NULL)",
            ),
            (
                "version_nullable",
                "CREATE TABLE peers (endpoint_id BLOB NOT NULL PRIMARY KEY)",
                "CREATE TABLE schema_version (version INTEGER)",
            ),
            (
                "version_primary_key",
                "CREATE TABLE peers (endpoint_id BLOB NOT NULL PRIMARY KEY)",
                "CREATE TABLE schema_version (version INTEGER NOT NULL PRIMARY KEY)",
            ),
            (
                "version_extra_column",
                "CREATE TABLE peers (endpoint_id BLOB NOT NULL PRIMARY KEY)",
                "CREATE TABLE schema_version (version INTEGER NOT NULL, extra TEXT)",
            ),
        ];
        let dir = tempfile::tempdir().unwrap();
        for (case, peers_sql, version_sql) in cases {
            let path = dir.path().join(format!("{case}.db"));
            let db = Builder::new_local(path.to_str().unwrap())
                .build()
                .await
                .unwrap();
            let conn = db.connect().unwrap();
            conn.execute(version_sql, ()).await.unwrap();
            conn.execute(peers_sql, ()).await.unwrap();
            conn.execute("INSERT INTO schema_version(version) VALUES (1)", ())
                .await
                .unwrap();
            let peer = test_endpoint_id(20);
            let peer_column = if case == "peer_missing_column" {
                "other"
            } else {
                "endpoint_id"
            };
            conn.execute(
                format!("INSERT INTO peers({peer_column}) VALUES (?1)"),
                [peer.as_bytes().to_vec()],
            )
            .await
            .unwrap();
            let schema_before = {
                let mut rows = conn
                    .query(
                        "SELECT name, sql FROM sqlite_master WHERE type='table' ORDER BY name",
                        (),
                    )
                    .await
                    .unwrap();
                let mut schema = Vec::new();
                while let Some(row) = rows.next().await.unwrap() {
                    schema.push((row.get::<String>(0).unwrap(), row.get::<String>(1).unwrap()));
                }
                schema
            };
            drop(conn);
            drop(db);

            let error = match AppState::open(&path, &TestPrivateFileWriter).await {
                Ok(_) => panic!("{case}: incompatible schema was accepted"),
                Err(error) => error,
            };
            let message = format!("{error:#}");
            assert!(
                message.contains(case.split('_').next().unwrap())
                    || message.contains("incompatible"),
                "{case}: {message}"
            );

            let db = Builder::new_local(path.to_str().unwrap())
                .build()
                .await
                .unwrap();
            let conn = db.connect().unwrap();
            let mut rows = conn
                .query(
                    "SELECT name, sql FROM sqlite_master WHERE type='table' ORDER BY name",
                    (),
                )
                .await
                .unwrap();
            let mut schema_after = Vec::new();
            while let Some(row) = rows.next().await.unwrap() {
                schema_after.push((row.get::<String>(0).unwrap(), row.get::<String>(1).unwrap()));
            }
            assert_eq!(schema_after, schema_before, "{case}: schema was modified");
            let mut rows = conn
                .query("SELECT quote(version) FROM schema_version", ())
                .await
                .unwrap();
            assert_eq!(
                rows.next()
                    .await
                    .unwrap()
                    .unwrap()
                    .get::<String>(0)
                    .unwrap(),
                if case == "version_type" { "'1'" } else { "1" }
            );
            assert!(rows.next().await.unwrap().is_none());
            let mut peers = conn
                .query(format!("SELECT hex({peer_column}) FROM peers"), ())
                .await
                .unwrap();
            assert_eq!(
                peers
                    .next()
                    .await
                    .unwrap()
                    .unwrap()
                    .get::<String>(0)
                    .unwrap(),
                hex::encode_upper(peer.as_bytes()),
                "{case}: peer data was modified",
            );
            assert!(peers.next().await.unwrap().is_none());
        }
    }

    #[tokio::test]
    async fn accepts_existing_schema_with_lowercase_declared_types() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("lowercase.db");
        let db = Builder::new_local(path.to_str().unwrap())
            .build()
            .await
            .unwrap();
        let conn = db.connect().unwrap();
        conn.execute("CREATE TABLE schema_version (version integer NOT NULL)", ())
            .await
            .unwrap();
        conn.execute(
            "CREATE TABLE peers (endpoint_id blob NOT NULL PRIMARY KEY)",
            (),
        )
        .await
        .unwrap();
        conn.execute("INSERT INTO schema_version(version) VALUES (1)", ())
            .await
            .unwrap();
        let peer = test_endpoint_id(21);
        conn.execute(
            "INSERT INTO peers(endpoint_id) VALUES (?1)",
            [peer.as_bytes().to_vec()],
        )
        .await
        .unwrap();
        drop(conn);
        drop(db);
        let state = AppState::open(&path, &TestPrivateFileWriter).await.unwrap();
        assert_eq!(state.load_peers().await.unwrap(), vec![peer]);
    }

    #[tokio::test]
    async fn invalid_legacy_input_does_not_modify_state() {
        let dir = tempfile::tempdir().unwrap();
        let state = AppState::open(&dir.path().join("state.db"), &TestPrivateFileWriter)
            .await
            .unwrap();
        assert!(
            state
                .import_legacy_peers(br#"["not-an-endpoint"]"#)
                .await
                .is_err()
        );
        assert!(state.load_peers().await.unwrap().is_empty());
    }
}
