require('dotenv').config();
const express = require('express');
const app = express();
const PORT = process.env.PORT || 3000;

app.use(express.json());

app.get('/health', (req, res) => {
  res.json({ status: 'ok', service: 'Pstep Gateway' });
});

app.listen(PORT, () => {
  console.log(`Pstep gateway listening on port ${PORT}`);
});