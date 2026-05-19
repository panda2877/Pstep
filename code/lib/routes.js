const { handleWithFallback } = require('./fallback.js');
const { db } = require('./db.js');

function setupRoutes(app) {
  app.post('/v1/chat/completions', async (req, res) => {
    try {
      const result = await handleWithFallback(req.body);
      res.json(result.data);
    } catch (err) {
      res.status(502).json({ error: err.message });
    }
  });

  app.get('/health', (req, res) => {
    res.json({ status: 'ok', service: 'Pstep Gateway' });
  });

  app.get('/stats', (req, res) => {
    db.all(`SELECT * FROM token_usage ORDER BY id DESC LIMIT 10`, (err, rows) => {
      if (err) res.status(500).json({ error: err.message });
      else res.json(rows);
    });
  });
}

module.exports = { setupRoutes };