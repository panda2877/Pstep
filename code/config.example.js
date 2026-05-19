// config.example.js - 复制此文件为 config.js 并填入真实值
module.exports = {
  models: {
    'mimo-v2.5': {
      url: 'https://token-plan-cn.xiaomimimo.com/v1/chat/completions',
      apiKey: 'tp-cfq4whtghewq55sn5hn8dxayt6yfgknlk2m5s7eso5ve5lef',
      remoteModel: 'mimo-v2.5',
    },
    // 可以添加更多模型
  },
  fallbackChains: {
    default: ['mimo-v2.5'],
  },
  port: process.env.PORT || 3000,
};