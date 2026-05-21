# CLAUDE.md

This file provides guidance to Claude Code (claude.ai/code) when working with code in this repository.

## Project Overview

Pstep is an AI model gateway/proxy service written in Rust. It exposes an OpenAI-compatible API (`/v1/chat/completions`) and a WebSocket-based ACP (Agent Client Protocol) server, routing requests to configured LLM providers with fallback chain support. Token usage is logged to SQLite.

## Commands

```bash
# 构建整个 workspace
cargo build

# 运行所有测试
cargo test

# 运行特定模块测试
cargo test -p pstep-core -- config    # 配置模块测试
cargo test -p pstep-core -- client    # 模型调用测试

# Lint 检查
cargo clippy --workspace

# 格式检查
cargo fmt --all --check

# Release 构建（单个可执行文件）
cargo build --release
```

## Architecture

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

## CI / CNB 云原生构建

每次 push 到任意分支时自动执行（配置在 `.cnb.yml`）：

| Job | 说明 | 容器 |
|-----|------|------|
| `build-and-test` | `cargo build` → `cargo test` → `cargo clippy` → `cargo fmt` | `rust:latest` |
| `release-build` | `cargo build --release`（Release 二进制） | `rust:latest` |
| `sync-to-github` | 同步代码到 GitHub（`panda2877/Pstep`） | `alpine/git` |

- clippy 和 fmt 为 `continue-on-error: true`，不阻断流水线
- 需安装 `pkg-config` + `libssl-dev`（reqwest native-tls 依赖）

## Key Conventions

- Rust: edition 2024.
- Environment variables are used for API keys — the `apiKeyEnv` field in `config/models.json` maps to environment variables. A `.env` file (gitignored) can provide these.
- The gateway is OpenAI-compatible — any client that speaks the `/v1/chat/completions` protocol can use it.
- Database file `stats.db` is gitignored. It is auto-created with the `token_usage` table on first run.
