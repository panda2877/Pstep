const sqlite3 = require('sqlite3').verbose();
const path = require('path');

const dbPath = path.resolve(__dirname, '../../stats.db'); // 注意路径
const db = new sqlite3.Database(dbPath);

db.serialize(() => {
  db.run(`
    CREATE TABLE IF NOT EXISTS token_usage (
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
    )
  `);
});

function logUsage(data) {
  const { model, requestedModel, success, inputTokens, outputTokens, totalTokens, latencyMs, error } = data;
  console.log('[STATS]', { model, requestedModel, success, inputTokens, outputTokens, totalTokens, latencyMs, error });
  db.run(
    `INSERT INTO token_usage (model, requested_model, success, input_tokens, output_tokens, total_tokens, latency_ms, error)
     VALUES (?, ?, ?, ?, ?, ?, ?, ?)`,
    [model, requestedModel, success ? 1 : 0, inputTokens, outputTokens, totalTokens, latencyMs, error],
    (err) => { if (err) console.error('DB insert error:', err); }
  );
}

module.exports = { db, logUsage };