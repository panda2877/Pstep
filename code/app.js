const path = require('path');
require('dotenv').config({ path: path.join(__dirname, '.env') });
const express = require('express');
const { setupRoutes } = require('./services/gateway/routes');
const { SERVER_PORT } = require('./config');

const app = express();
app.use(express.json());
setupRoutes(app);

app.listen(SERVER_PORT, () => {
  console.log(`Pstep gateway listening on port ${SERVER_PORT}`);
});