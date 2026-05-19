const axios = require('axios');
const { MODEL_CONFIG } = require('../../config');

async function callModel(modelName, requestBody) {
  const modelConfig = MODEL_CONFIG[modelName];
  if (!modelConfig) throw new Error(`Model ${modelName} not configured`);

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
        stream: false,
        model: modelConfig.remoteModel || modelName,
      },
      timeout: 30000,
    });
    const latency = Date.now() - start;
    const usage = response.data.usage || {};
    const input_tokens = usage.prompt_tokens || usage.input_tokens || 0;
    const output_tokens = usage.completion_tokens || usage.output_tokens || 0;
    const total_tokens = usage.total_tokens || (input_tokens + output_tokens) || 0;

    return {
      success: true,
      usage: { input_tokens, output_tokens, total_tokens },
      latency,
      model: modelName,
      data: response.data,
    };
  } catch (err) {
    const latency = Date.now() - start;
    console.error(`Model ${modelName} failed:`, err.message);
    return {
      success: false,
      error: err.message,
      latency,
      model: modelName,
    };
  }
}

module.exports = { callModel };