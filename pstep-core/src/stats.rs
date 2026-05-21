use rusqlite::{params, Connection};
use std::path::Path;
use std::sync::{Arc, Mutex};

#[derive(Clone)]
pub struct StatsDb {
    conn: Arc<Mutex<Connection>>,
}

#[derive(Debug, serde::Serialize)]
pub struct UsageRecord {
    pub id: i64,
    pub timestamp: String,
    pub model: String,
    pub requested_model: String,
    pub success: bool,
    pub input_tokens: i64,
    pub output_tokens: i64,
    pub total_tokens: i64,
    pub latency_ms: i64,
    pub error: Option<String>,
}

impl StatsDb {
    pub fn open(db_path: &Path) -> Result<Self, rusqlite::Error> {
        let conn = Connection::open(db_path)?;
        conn.execute_batch(
            "CREATE TABLE IF NOT EXISTS token_usage (
                id INTEGER PRIMARY KEY AUTOINCREMENT,
                timestamp DATETIME DEFAULT CURRENT_TIMESTAMP,
                model TEXT,
                requested_model TEXT,
                success INTEGER,
                input_tokens INTEGER,
                output_tokens INTEGER,
                total_tokens INTEGER,
                latency_ms INTEGER,
                error TEXT
            );",
        )?;
        Ok(Self {
            conn: Arc::new(Mutex::new(conn)),
        })
    }

    #[allow(clippy::too_many_arguments)]
    pub fn log_usage(
        &self,
        model: &str,
        requested_model: &str,
        success: bool,
        input_tokens: u32,
        output_tokens: u32,
        total_tokens: u32,
        latency_ms: u64,
        error: Option<&str>,
    ) {
        let conn = self.conn.lock().unwrap();
        conn.execute(
            "INSERT INTO token_usage (model, requested_model, success, input_tokens, output_tokens, total_tokens, latency_ms, error)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)",
            params![
                model,
                requested_model,
                success as i32,
                input_tokens as i64,
                output_tokens as i64,
                total_tokens as i64,
                latency_ms as i64,
                error,
            ],
        )
        .unwrap_or_else(|e| {
            tracing::error!(error = %e, "failed to log usage");
            0
        });
    }

    pub fn recent(&self, limit: usize) -> Vec<UsageRecord> {
        let conn = self.conn.lock().unwrap();
        let mut stmt = conn
            .prepare(
                "SELECT id, timestamp, model, requested_model, success, input_tokens, output_tokens, total_tokens, latency_ms, error
                 FROM token_usage ORDER BY id DESC LIMIT ?1",
            )
            .unwrap();

        stmt.query_map(params![limit as i64], |row| {
            Ok(UsageRecord {
                id: row.get(0)?,
                timestamp: row.get(1)?,
                model: row.get(2)?,
                requested_model: row.get(3)?,
                success: row.get::<_, i32>(4)? != 0,
                input_tokens: row.get(5)?,
                output_tokens: row.get(6)?,
                total_tokens: row.get(7)?,
                latency_ms: row.get(8)?,
                error: row.get(9)?,
            })
        })
        .unwrap()
        .filter_map(|r| r.ok())
        .collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::tempdir;

    #[test]
    fn log_and_recent_single_record() {
        let dir = tempdir().unwrap();
        let db = StatsDb::open(&dir.path().join("test.db")).unwrap();

        db.log_usage("gpt-4", "gpt-3.5-turbo", true, 10, 20, 30, 150, None);

        let records = db.recent(10);
        assert_eq!(records.len(), 1);

        let r = &records[0];
        assert_eq!(r.model, "gpt-4");
        assert_eq!(r.requested_model, "gpt-3.5-turbo");
        assert!(r.success);
        assert_eq!(r.input_tokens, 10);
        assert_eq!(r.output_tokens, 20);
        assert_eq!(r.total_tokens, 30);
        assert_eq!(r.latency_ms, 150);
        assert!(r.error.is_none());
    }

    #[test]
    fn log_and_recent_multiple_records() {
        let dir = tempdir().unwrap();
        let db = StatsDb::open(&dir.path().join("test.db")).unwrap();

        db.log_usage("model-a", "requested-a", true, 10, 20, 30, 100, None);
        db.log_usage("model-b", "requested-b", false, 5, 5, 10, 200, Some("timeout"));
        db.log_usage("model-c", "requested-c", true, 50, 100, 150, 50, None);

        let records = db.recent(10);
        assert_eq!(records.len(), 3);

        // recent() returns in DESC order (newest first)
        assert_eq!(records[0].model, "model-c");
        assert_eq!(records[1].model, "model-b");
        assert_eq!(records[2].model, "model-a");
    }

    #[test]
    fn log_with_error_records_error_field() {
        let dir = tempdir().unwrap();
        let db = StatsDb::open(&dir.path().join("test.db")).unwrap();

        db.log_usage("model-a", "model-a", false, 0, 0, 0, 0, Some("connection refused"));

        let records = db.recent(10);
        assert_eq!(records.len(), 1);
        assert!(!records[0].success);
        assert_eq!(records[0].error.as_deref(), Some("connection refused"));
    }

    #[test]
    fn recent_respects_limit() {
        let dir = tempdir().unwrap();
        let db = StatsDb::open(&dir.path().join("test.db")).unwrap();

        for i in 0..5u32 {
            db.log_usage("model", "model", true, i, i, i, i as u64, None);
        }

        let records = db.recent(3);
        assert_eq!(records.len(), 3);

        // Should be the last 3 (id 5, 4, 3)
        assert_eq!(records[0].id, 5);
        assert_eq!(records[1].id, 4);
        assert_eq!(records[2].id, 3);
    }

    #[test]
    fn recent_empty_db_returns_empty() {
        let dir = tempdir().unwrap();
        let db = StatsDb::open(&dir.path().join("test.db")).unwrap();

        let records = db.recent(10);
        assert!(records.is_empty());
    }

    #[test]
    fn db_auto_creates_table() {
        let dir = tempdir().unwrap();
        let db_path = dir.path().join("test.db");

        // First open creates the table
        let db1 = StatsDb::open(&db_path).unwrap();
        db1.log_usage("model", "model", true, 1, 1, 1, 1, None);
        drop(db1);

        // Second open reuses existing table
        let db2 = StatsDb::open(&db_path).unwrap();
        let records = db2.recent(10);
        assert_eq!(records.len(), 1);
        assert_eq!(records[0].model, "model");
    }
}
