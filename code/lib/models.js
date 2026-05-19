const axios = require('axios');
const { MODEL_CONFIG } = require('./config');

async function callModel(modelName, requestBody) {
  const modelConfig = MODEL_CONFIG[modelName];
  if (!modelConfig) {
    throw new Error(`Model ${modelName} not configured`);
  }

  const start = Date.now();
  try {
    const response = await axios({
      method: 'post',
      url: modelConfig.url,
      headers: {
        'Content-Type': 'application/json',
        'Authorization': `Bearer ${modelConfig.apiKey}`
      },
      data: {
        ...requestBody,
        model: modelConfig.remoteModel || modelName,
      },
      timeout: 30000,
    });
    const latency = Date.now() - start;
    const usage = response.data.usage || { prompt_tokens: 0, completion_tokens: 0, total_tokens: 0 };
    return {
      success: true,
      usage: {
        input_tokens: usage.prompt_tokens || 0,
        output_tokens: usage.completion_tokens || 0,
        total_tokens: usage.total_tokens || 0,
      },
      latency,
      model: modelName,
      data: response.data,
    };
  } catch (err) {
    const latency = Date.now() - start;
    console.error(`Model ${modelName} failed:`, err.message);
    return {
      success: false,
      error: err.message || 'Unknown error',
      latency,
      model: modelName,
    };
  }
}

module.exports = { callModel };