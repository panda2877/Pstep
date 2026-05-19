const axios = require('axios');
const { PassThrough } = require('stream');
const { MODEL_CONFIG } = require('../../config');

async function callModelStream(modelName, requestBody) {
  const modelConfig = MODEL_CONFIG[modelName];
  if (!modelConfig) throw new Error(`Model ${modelName} not configured`);

  const start = Date.now();
  let finalUsage = null;
  const proxyStream = new PassThrough();

  try {
    const response = await axios({
      method: 'post',
      url: modelConfig.url,
      headers: {
        'Content-Type': 'application/json',
        'Authorization': `Bearer ${modelConfig.apiKey}`,
        'Accept': 'text/event-stream',
      },
      data: {
        ...requestBody,
        stream: true,
        model: modelConfig.remoteModel || modelName,
      },
      responseType: 'stream',
      timeout: 30000,
    });

    response.data.on('data', (chunk) => {
      const chunkStr = chunk.toString();
      const lines = chunkStr.split('\n');
      for (const line of lines) {
        if (line.startsWith('data: ')) {
          const data = line.slice(6);
          if (data === '[DONE]') continue;
          try {
            const parsed = JSON.parse(data);
            if (parsed.usage) {
              finalUsage = parsed.usage;
            }
          } catch (e) {}
        }
      }
      proxyStream.write(chunk);
    });

    response.data.on('end', () => {
      const latency = Date.now() - start;
      const usage = finalUsage || {};
      const input_tokens = usage.prompt_tokens || usage.input_tokens || 0;
      const output_tokens = usage.completion_tokens || usage.output_tokens || 0;
      const total_tokens = usage.total_tokens || (input_tokens + output_tokens) || 0;
      proxyStream.emit('stats', {
        success: true,
        usage: { input_tokens, output_tokens, total_tokens },
        latency,
        model: modelName,
      });
      proxyStream.end();
    });

    response.data.on('error', (err) => {
      proxyStream.destroy(err);
    });

    return { stream: proxyStream };
  } catch (err) {
    const latency = Date.now() - start;
    proxyStream.destroy(err);
    return { error: err.message, latency, model: modelName };
  }
}

module.exports = { callModelStream };