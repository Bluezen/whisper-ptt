use anyhow::{Context, Result};
use rusqlite::Connection;
use std::path::Path;

const SCHEMA_VERSION: i32 = 1;

pub struct History {
    conn: Connection,
}

impl History {
    /// Open (or create) the database at the given path.
    pub fn open(path: &Path) -> Result<Self> {
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        let conn = Connection::open(path)
            .with_context(|| format!("failed to open database: {}", path.display()))?;

        conn.execute_batch("PRAGMA journal_mode=WAL;")?;

        conn.execute_batch(
            "CREATE TABLE IF NOT EXISTS schema_version (
                version INTEGER NOT NULL
            );
            CREATE TABLE IF NOT EXISTS transcriptions (
                id INTEGER PRIMARY KEY AUTOINCREMENT,
                text TEXT NOT NULL,
                language TEXT,
                model TEXT NOT NULL,
                duration_ms INTEGER,
                created_at TEXT NOT NULL
            );",
        )?;

        // Set schema version if empty
        let count: i32 =
            conn.query_row("SELECT COUNT(*) FROM schema_version", [], |row| row.get(0))?;
        if count == 0 {
            conn.execute(
                "INSERT INTO schema_version (version) VALUES (?1)",
                [SCHEMA_VERSION],
            )?;
        }

        Ok(Self { conn })
    }

    /// Insert a transcription record.
    pub fn insert(
        &self,
        text: &str,
        language: Option<&str>,
        model: &str,
        duration_ms: u64,
    ) -> Result<()> {
        let now = chrono::Utc::now().to_rfc3339();
        self.conn.execute(
            "INSERT INTO transcriptions (text, language, model, duration_ms, created_at)
             VALUES (?1, ?2, ?3, ?4, ?5)",
            rusqlite::params![text, language, model, duration_ms as i64, now],
        )?;
        Ok(())
    }

    /// Get the count of transcriptions (for testing).
    #[cfg(test)]
    pub fn count(&self) -> Result<i64> {
        let count: i64 = self
            .conn
            .query_row("SELECT COUNT(*) FROM transcriptions", [], |row| row.get(0))?;
        Ok(count)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_open_creates_tables() {
        let dir = tempfile::tempdir().unwrap();
        let db_path = dir.path().join("test.db");
        let history = History::open(&db_path).unwrap();
        assert_eq!(history.count().unwrap(), 0);
    }

    #[test]
    fn test_insert_and_count() {
        let dir = tempfile::tempdir().unwrap();
        let db_path = dir.path().join("test.db");
        let history = History::open(&db_path).unwrap();

        history
            .insert("hello world", Some("en"), "tiny", 1500)
            .unwrap();
        assert_eq!(history.count().unwrap(), 1);

        history
            .insert("bonjour le monde", Some("fr"), "large-v3-turbo", 2300)
            .unwrap();
        assert_eq!(history.count().unwrap(), 2);
    }

    #[test]
    fn test_insert_with_no_language() {
        let dir = tempfile::tempdir().unwrap();
        let db_path = dir.path().join("test.db");
        let history = History::open(&db_path).unwrap();

        history.insert("test", None, "base", 800).unwrap();
        assert_eq!(history.count().unwrap(), 1);
    }

    #[test]
    fn test_schema_version() {
        let dir = tempfile::tempdir().unwrap();
        let db_path = dir.path().join("test.db");
        let history = History::open(&db_path).unwrap();

        let version: i32 = history
            .conn
            .query_row("SELECT version FROM schema_version", [], |row| row.get(0))
            .unwrap();
        assert_eq!(version, 1);
    }

    #[test]
    fn test_wal_mode() {
        let dir = tempfile::tempdir().unwrap();
        let db_path = dir.path().join("test.db");
        let history = History::open(&db_path).unwrap();

        let mode: String = history
            .conn
            .query_row("PRAGMA journal_mode", [], |row| row.get(0))
            .unwrap();
        assert_eq!(mode, "wal");
    }

    #[test]
    fn test_reopen_existing_db() {
        let dir = tempfile::tempdir().unwrap();
        let db_path = dir.path().join("test.db");

        {
            let history = History::open(&db_path).unwrap();
            history.insert("first", Some("en"), "tiny", 1000).unwrap();
        }
        {
            let history = History::open(&db_path).unwrap();
            assert_eq!(history.count().unwrap(), 1);
            history.insert("second", Some("fr"), "tiny", 1200).unwrap();
            assert_eq!(history.count().unwrap(), 2);
        }
    }
}
