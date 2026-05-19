const express = require('express');
const { PORT } = require('./lib/config.js');
const { setupRoutes } = require('./lib/routes.js');


const app = express();
app.use(express.json());

setupRoutes(app);

app.listen(PORT, () => {
  console.log(`Pstep gateway listening on port ${PORT}`);
});