use rusqlite::{params, Connection};
use std::path::Path;
use std::sync::Mutex;

pub struct StatsDb {
    conn: Mutex<Connection>,
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
            conn: Mutex::new(conn),
        })
    }

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
