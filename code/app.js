require('dotenv').config();
const express = require('express');
const { spawn } = require('child_process');
const path = require('path');
const { setupRoutes } = require('./services/gateway/routes');
const { SERVER_PORT } = require('./config');

const app = express();
app.use(express.json());
setupRoutes(app);

// 启动 HTTP 网关
const server = app.listen(SERVER_PORT, () => {
  console.log(`Pstep gateway listening on port ${SERVER_PORT}`);
});

// 启动 ACP 服务器（pi-acp）
const piAcp = spawn('npx', ['pi-acp', '--port', '3400'], {
  cwd: __dirname,
  stdio: 'inherit',
  env: {
    ...process.env,
    PI_CODING_AGENT_DIR: path.join(__dirname, 'config', 'pi'),
  }
});

piAcp.on('error', (err) => {
  console.error('pi-acp failed to start:', err);
  server.close();
  process.exit(1);
});

// 优雅关闭
process.on('SIGINT', () => {
  console.log('\nShutting down...');
  piAcp.kill('SIGINT');
  server.close(() => process.exit(0));
});