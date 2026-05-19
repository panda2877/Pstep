const path = require('path');
const fs = require('fs');

const configPath = path.join(__dirname, 'models.json');
let rawConfig = { models: {}, fallbackChains: {}, server: { port: 3000 } };

if (fs.existsSync(configPath)) {
  rawConfig = JSON.parse(fs.readFileSync(configPath, 'utf-8'));
} else {
  console.warn('config/models.json not found, using defaults');
}

// 将模型配置中的 apiKeyEnv 替换为实际的环境变量值
const MODEL_CONFIG = {};
for (const [name, model] of Object.entries(rawConfig.models)) {
  MODEL_CONFIG[name] = {
    ...model,
    apiKey: process.env[model.apiKeyEnv] || model.apiKey || ''
  };
}

module.exports = {
  MODEL_CONFIG,
  FALLBACK_CHAINS: rawConfig.fallbackChains,
  SERVER_PORT: rawConfig.server.port,
};