const { callModel, callModelStream } = require('../models');
const { logUsage } = require('../stats/db');
const { FALLBACK_CHAINS } = require('../../config');

async function handleWithFallback(requestBody, res) {
  const requestedModel = requestBody.model || 'default';
  const chain = FALLBACK_CHAINS[requestedModel] || FALLBACK_CHAINS.default || [];
  if (chain.length === 0) throw new Error('No models in fallback chain');
  const isStream = requestBody.stream === true;

  if (isStream) {
    const model = chain[0];
    const result = await callModelStream(model, requestBody);
    if (result.error) {
      logUsage({
        model: result.model,
        requestedModel,
        success: false,
        inputTokens: 0, outputTokens: 0, totalTokens: 0,
        latencyMs: Math.round(result.latency),
        error: result.error
      });
      throw new Error(`Stream model failed: ${result.error}`);
    }
    result.stream.on('stats', (stats) => {
      logUsage({
        model: stats.model,
        requestedModel,
        success: stats.success,
        inputTokens: stats.usage.input_tokens,
        outputTokens: stats.usage.output_tokens,
        totalTokens: stats.usage.total_tokens,
        latencyMs: Math.round(stats.latency),
        error: null
      });
    });
    return { stream: result.stream };
  } else {
    let lastError = null;
    for (const model of chain) {
      const result = await callModel(model, requestBody);
      logUsage({
        model: result.model,
        requestedModel,
        success: result.success,
        inputTokens: result.usage?.input_tokens || 0,
        outputTokens: result.usage?.output_tokens || 0,
        totalTokens: result.usage?.total_tokens || 0,
        latencyMs: Math.round(result.latency),
        error: result.error
      });
      if (result.success) return { data: result.data };
      lastError = result.error;
    }
    throw new Error(`All models failed: ${lastError}`);
  }
}

module.exports = { handleWithFallback };