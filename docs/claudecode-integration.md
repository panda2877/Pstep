# Claude Code 对接指南

## 概述

Pstep 网关支持 Anthropic Messages API 兼容格式，可作为 Claude Code 的后端代理。
请求链路：`Claude Code → Pstep Gateway (port 3000) → 上游模型 API`

## 配置步骤

### 1. 启动网关

```bash
cd ~/repos/pstep
cargo run -p pstep-gateway
```

确认网关运行：
```bash
curl http://localhost:3000/health
# 返回 {"status":"ok","service":"Pstep Gateway"}
```

### 2. 配置 Claude Code

编辑 `~/.claude/settings.json`：

```json
{
  "env": {
    "ANTHROPIC_BASE_URL": "http://localhost:3000",
    "ANTHROPIC_AUTH_TOKEN": "你的API密钥",
    "ANTHROPIC_MODEL": "模型名称"
  }
}
```

- `ANTHROPIC_BASE_URL`：网关地址，**不要**加 `/v1` 后缀
- `ANTHROPIC_AUTH_TOKEN`：与 `config/models.json` 中的 `apiKey` 一致
- `ANTHROPIC_MODEL`：与 `config/models.json` 中的模型 key 一致

### 3. 启动 Claude Code

```bash
claude
```

## 模型配置

`config/models.json` 示例：

```json
{
  "models": {
    "mimo-v2.5": {
      "url": "https://token-plan-cn.xiaomimimo.com/v1/chat/completions",
      "apiKey": "你的API密钥",
      "remoteModel": "mimo-v2.5"
    }
  },
  "fallbackChains": {
    "default": ["mimo-v2.5"]
  },
  "server": {
    "port": 3000,
    "wsPort": 3400
  }
}
```

## 双协议支持

网关同时暴露两个 API 格式：

| 端点 | 格式 | 用途 |
|------|------|------|
| `POST /v1/chat/completions` | OpenAI 格式 | 其他 OpenAI 兼容客户端 |
| `POST /v1/messages` | Anthropic 格式 | Claude Code |

两个端点共用同一个 fallback chain 和模型配置。

## 环境变量

| 变量 | 说明 | 示例 |
|------|------|------|
| `ANTHROPIC_BASE_URL` | 网关地址（不含 /v1） | `http://localhost:3000` |
| `ANTHROPIC_AUTH_TOKEN` | API 密钥 | `tp-xxxx` |
| `ANTHROPIC_MODEL` | 默认模型名 | `mimo-v2.5` |
| `PSTEP_CONFIG_DIR` | 配置目录（可选） | `config` |

## 故障排查

### Claude Code 无法连接

```bash
# 1. 确认网关在运行
curl http://localhost:3000/health

# 2. 确认模型列表
curl http://localhost:3000/v1/models

# 3. 直接测试 Anthropic 端点
curl http://localhost:3000/v1/messages \
  -H "Content-Type: application/json" \
  -H "x-api-key: 你的密钥" \
  -H "anthropic-version: 2023-06-01" \
  -d '{"model":"mimo-v2.5","max_tokens":50,"messages":[{"role":"user","content":"hi"}]}'

# 4. 查看网关日志
tail -f /tmp/pstep-gw.log
```

### 恢复直连 Anthropic API

将 `~/.claude/settings.json` 改回：

```json
{
  "env": {
    "ANTHROPIC_BASE_URL": "https://api.anthropic.com",
    "ANTHROPIC_AUTH_TOKEN": "你的Anthropic密钥",
    "ANTHROPIC_MODEL": "claude-sonnet-4-20250514"
  }
}
```
