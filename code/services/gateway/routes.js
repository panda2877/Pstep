const { handleWithFallback } = require('./fallback');
const { db } = require('../stats/db');

function setupRoutes(app) {
  app.post('/v1/chat/completions', async (req, res) => {
    try {
      const result = await handleWithFallback(req.body, res);
      if (result.stream) {
        res.setHeader('Content-Type', 'text/event-stream');
        res.setHeader('Cache-Control', 'no-cache');
        res.setHeader('Connection', 'keep-alive');
        result.stream.pipe(res);
        result.stream.on('error', (err) => {
          if (!res.headersSent) res.status(500).json({ error: err.message });
        });
      } else {
        res.json(result.data);
      }
    } catch (err) {
      res.status(502).json({ error: err.message });
    }
  });

  app.get('/health', (req, res) => res.json({ status: 'ok', service: 'Pstep Gateway' }));
  app.get('/stats', (req, res) => {
    db.all(`SELECT * FROM token_usage ORDER BY id DESC LIMIT 10`, (err, rows) => {
      if (err) res.status(500).json({ error: err.message });
      else res.json(rows);
    });
  });
}

module.exports = { setupRoutes };