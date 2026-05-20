# CLAUDE.md

This file provides guidance to Claude Code (claude.ai/code) when working with code in this repository.

## Project Overview

Pstep is an AI model gateway/proxy service. It exposes an OpenAI-compatible API (`/v1/chat/completions`) and a WebSocket-based ACP (Agent Client Protocol) server, routing requests to configured LLM providers with fallback chain support. Token usage is logged to SQLite.

**正在从 Node.js 重构为 Rust**，详见 `docs/rust-rewrite-design.md` 和 `docs/kanban.md`。

## Commands

### Rust（主要开发方向）

```bash
# 构建整个 workspace
cargo build

# 运行所有测试
cargo test

# 运行特定模块测试
cargo test -p pstep-core -- config    # 配置模块测试
cargo test -p pstep-core -- client    # 模型调用测试

# Release 构建（单个可执行文件）
cargo build --release
```

### Node.js（保留，过渡期可用）

```bash
cd code && npm install
node app.js          # 启动网关 (port 3000 + 3400)
node start-all.js    # 同时启动网关 + Pi 智能体
```

## Architecture

### Rust Workspace（当前开发重点）

```
pstep-core/     # 核心层 (lib crate)
├── config.rs   # 配置解析，apiKeyEnv→环境变量映射
├── client.rs   # reqwest 模型调用，非流式 + 流式 SSE
├── fallback.rs # Fallback chain 引擎
├── stats.rs    # SQLite 用量统计
└── manager.rs  # ModelStore 运行时 CRUD

pstep-gateway/  # 网关层 (bin crate)
└── main.rs     # Axum HTTP 服务器 + 路由
```

### Node.js（旧版，在 `code/` 目录）

- **`app.js`** — Entry point. Starts an Express HTTP server (port 3000) and a WebSocket server (port 3400) for ACP JSON-RPC messages.
- **`start-all.js`** — Launches both `app.js` (gateway) and `npx pi` (Pi coding agent) as child processes, passing `PI_CODING_AGENT_DIR` to point Pi at `config/pi/`.
- **`config/index.js`** — Reads `config/models.json`, resolves API keys from environment variables via `apiKeyEnv` fields, and exports `MODEL_CONFIG`, `FALLBACK_CHAINS`, and `SERVER_PORT`.
- **`config/models.json`** — Defines available models (URL, API key env var, remote model name) and fallback chains.
- **`config/pi/`** — Configuration for the Pi coding agent: `settings.json` (default provider/model), `models.json` (provider pointing to `localhost:3000/v1`), `auth.json`.
- **`services/gateway/routes.js`** — Express routes: `POST /v1/chat/completions` (main proxy endpoint), `GET /health`, `GET /stats`.
- **`services/gateway/fallback.js`** — Orchestrates model calls through fallback chains; supports both streaming and non-streaming. Logs each attempt via `logUsage`.
- **`services/models/base.js`** — Non-streaming model calls via axios to configured provider URLs.
- **`services/models/stream.js`** — Streaming (SSE) model calls; emits a `stats` event on completion with token usage data.
- **`services/stats/db.js`** — SQLite database (`stats.db` at project root) for token usage logging. Table: `token_usage`.

## Key Conventions

- Rust: edition 2024，CommonJS modules for Node.js (`"type": "commonjs"` in package.json).
- Environment variables are used for API keys — the `apiKeyEnv` field in `config/models.json` maps to `process.env` keys. A `.env` file (gitignored) can provide these.
- The Pi agent configuration in `config/pi/` is self-contained: it points at the local gateway as its LLM provider, so running `node start-all.js` gives Pi a working backend without external API keys.
- The gateway is OpenAI-compatible — any client that speaks the `/v1/chat/completions` protocol can use it.
- Database file `stats.db` is gitignored. It is auto-created with the `token_usage` table on first run.
