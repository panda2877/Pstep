const fs = require('fs');
const path = require('path');

const configPath = path.join(__dirname, '../config.json');
let config = {};

if (fs.existsSync(configPath)) {
  config = JSON.parse(fs.readFileSync(configPath, 'utf-8'));
} else {
  console.warn('config.json not found, using hardcoded fallback (development only)');
  config = {
    models: {
      'mimo-v2.5': {
        url: 'https://token-plan-cn.xiaomimimo.com/v1/chat/completions',
        apiKey: '你的API Key',   // 替换为真实Key
        remoteModel: 'mimo-v2.5'
      }
    },
    fallbackChains: { default: ['mimo-v2.5'] },
    server: { port: 3000 }
  };
}

module.exports = {
  MODEL_CONFIG: config.models,
  FALLBACK_CHAINS: config.fallbackChains,
  PORT: config.server.port,
};