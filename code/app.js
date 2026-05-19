require('dotenv').config();
const express = require('express');
const WebSocket = require('ws');
const { setupRoutes } = require('./services/gateway/routes');
const { SERVER_PORT } = require('./config');
const { callModel } = require('./services/models/base'); // 确保这个模块存在

const app = express();
app.use(express.json());
setupRoutes(app);
const httpServer = app.listen(SERVER_PORT, () => {
  console.log(`Pstep gateway listening on port ${SERVER_PORT}`);
});

const wss = new WebSocket.Server({ port: 3400 });
wss.on('connection', (ws) => {
  console.log('ACP client connected');
  ws.on('message', async (data) => {
    try {
      const msg = JSON.parse(data);
      const { id, method, params } = msg;
      if (method === 'initialize') {
        const response = {
          jsonrpc: '2.0',
          id,
          result: { capabilities: {} }
        };
        ws.send(JSON.stringify(response));
      } else if (method === 'prompt') {
        const messages = params.messages;
        const lastUserMsg = messages.filter(m => m.role === 'user').pop();
        const prompt = lastUserMsg ? lastUserMsg.content : '';
        const requestBody = {
          model: 'default',
          messages: [{ role: 'user', content: prompt }]
        };
        const result = await callModel('mimo-v2.5', requestBody);
        const content = result.success ? result.data.choices[0].message.content : `Error: ${result.error}`;
        const response = {
          jsonrpc: '2.0',
          id,
          result: { content }
        };
        ws.send(JSON.stringify(response));
      } else {
        // 其他方法返回错误
        ws.send(JSON.stringify({
          jsonrpc: '2.0',
          id,
          error: { code: -32601, message: 'Method not found' }
        }));
      }
    } catch (err) {
      console.error('Parse error:', err);
    }
  });
  ws.on('close', () => console.log('ACP client disconnected'));
});

console.log('ACP WebSocket server listening on port 3400');

process.on('SIGINT', () => {
  console.log('\nShutting down...');
  httpServer.close();
  wss.close();
  process.exit(0);
});