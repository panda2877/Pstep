const { callModel } = require('./models.js');
const { logUsage } = require('./db.js');
const { FALLBACK_CHAINS } = require('./config.js');

async function handleWithFallback(requestBody) {
  const requestedModel = requestBody.model || 'default';
  const chain = FALLBACK_CHAINS[requestedModel] || FALLBACK_CHAINS.default || [];
  
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
    if (result.success) return result;
    lastError = result.error;
  }
  throw new Error(`All models failed: ${lastError || 'no available models'}`);
}

module.exports = { handleWithFallback };