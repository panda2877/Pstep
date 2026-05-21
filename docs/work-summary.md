# 工作总结 — 2026-05-21

## 项目状态

Pstep（版本 0.3.0）正在从 Node.js 重构为 Rust，采用 workspace 分层架构（pstep-core lib + pstep-gateway bin）。**Rust 重构主体已基本完成**，共 42 个单元测试全部通过，端到端集成验证通过。

## 整体架构

```
Client (Claude Code / OpenAI 兼容客户端)
        │
        ├── POST /v1/chat/completions  (OpenAI 格式)
        ├── POST /v1/messages          (Anthropic Messages API 格式)
        └── WebSocket :3400            (ACP JSON-RPC 协议)
        │
pstep-gateway  (Axum HTTP Server + tokio-tungstenite WS Server)
        │
pstep-core     (lib crate)
        ├── config.rs   — GatewayConfig/ModelConfig 解析 + serde rename
        ├── client.rs   — ModelClient (reqwest) 非流式 + 流式 SSE
        ├── fallback.rs — Fallback chain 引擎（非流式 + 流式）
        ├── stats.rs    — SQLite token 用量统计
        └── manager.rs  — ModelStore (RwLock+HashMap) 运行时 CRUD
        │
    外部 LLM API (OpenAI / Anthropic / DeepSeek 等)
```

## 已完成的核心功能

### 项目骨架（全部完成）
- `pstep-core`（lib crate）和 `pstep-gateway`（bin crate）workspace 结构
- 依赖：tokio, axum, reqwest, serde, rusqlite, tracing, mockito 等
- `cargo build` 零 warning 通过；版本 0.3.0，edition 2024
- `config/` 移动到项目根，Rust 和 Node.js 共用

### pstep-core 核心层（全部完成）
| 模块 | 功能描述 | 测试数 |
|------|---------|--------|
| **config.rs** | GatewayConfig/ModelConfig serde 解析，apiKeyEnv→环境变量映射，`.env` 文件加载 | 13 ✅ |
| **client.rs** | ModelClient：`call_model()` 非流式 + `call_model_stream()` 流式 SSE，StreamHandle/StreamChunk 类型，自动 Bearer 认证 | 8 ✅ |
| **fallback.rs** | `handle_with_fallback()` 非流式 + `handle_stream_with_fallback()` 流式 fallback chain，响应 model 字段替换为实际模型名 | 6 ✅ |
| **stats.rs** | SQLite stats.db 自动建表（token_usage），`log_usage()` 写入，`recent(N)` 查询 | 6 ✅ |
| **manager.rs** | `ModelProvider` trait 抽象，`ModelStore`（RwLock+HashMap）运行时 CRUD，OpenAIProvider 适配，健康检查 | 8 ✅ |

测试合计：**42 个单元测试全部通过** ✅

### pstep-gateway 网关层（全部完成）

**HTTP 路由（端口 3000）：**
| 方法 | 路径 | 说明 |
|------|------|------|
| POST | `/v1/chat/completions` | OpenAI 兼容聊天补全（非流式 + 流式 SSE） |
| POST | `/v1/messages` | Anthropic Messages API 格式兼容 |
| GET | `/health` | `{ "status": "ok", "service": "Pstep Gateway" }` |
| GET | `/stats` | 最近 10 条 token 用量记录 |
| GET | `/v1/models` | 列出已注册模型（OpenAI 兼容格式） |
| POST | `/v1/models` | 注册/更新模型 |
| DELETE | `/v1/models/:name` | 删除模型 |

**WebSocket / ACP 服务器（端口 3400）:**
- 基于 `tokio-tungstenite` 的 JSON-RPC 协议处理
- `initialize` → 返回 capabilities
- `prompt` → 提取用户消息 → 调用 core fallback → 返回结果
- 未知方法 → 返回 `-32601 Method not found`
- 连接/断开日志记录

**Anthropic Messages API 兼容（`POST /v1/messages`）:**
- Anthropic content blocks ↔ OpenAI message 双向格式转换
- system prompt 传递（转为 OpenAI system message）
- tool_use/tool_result 消息格式转换（含 function calling 双向转换）
- 流式 SSE 事件链：`message_start` → `content_block_start` → `content_block_delta` → `content_block_stop` → `message_delta` → `message_stop`
- 非流式：返回 Anthropic 格式 JSON（content[], stop_reason, usage）

### 集成验证
- 启动网关 → 发送请求 → 验证响应格式（非流式 + 流式均通过）
- Anthropic 端点测试（非流式、流式、system prompt 全部正常）
- Claude Code 对接配置文档已完成（`docs/claudecode-integration.md`）

## 近期修复记录

### 2026-05-20：环境修复 + 代码修复
- 升级 Rust 工具链 1.75.0 → 1.95.0（支持 edition 2024）
- 配置 Cargo 镜像源（rsproxy.cn sparse index）
- 安装 OpenSSL 开发库
- 修复 SSE 解析器：重写为基于 buffer 的逐行解析，支持多消息单 chunk
- 修复 tokio runtime 冲突：使用 `Server::new_async().await`

### 2026-05-21：Serde rename + Anthropic 端点
- 修复 `ModelConfig.api_key` 缺少 `#[serde(rename = "apiKey")]` 导致 401 的问题
- 修复 `ServerConfig.ws_port` 缺少 `#[serde(rename = "wsPort")]` 的问题
- 完成 `POST /v1/messages` 和 Anthropic ↔ OpenAI 格式转换
- 端到端测试（非流式 + 流式）全部通过

## 待完成项

### 验证与测试
- [ ] Claude Code 完整多轮对话验证（需配置 `~/.claude/settings.json` 指向网关）
- [ ] 多模型 fallback 场景验证（至少 2 个模型，含主模型故障切换）
- [ ] 验证 `/stats` 中 `model` vs `requested_model` 记录正确
- [ ] ACP WebSocket 连接测试
- [ ] pstep-gateway 单元测试（路由处理、ACP 协议解析）
- [ ] `/v1/messages` 端点的 `max_tokens` 字段已接收但未传给上游（使用上游默认值）

### 生产化
- [ ] 从 SSE 流提取 usage 信息（`StreamSseChunk.usage` 字段未使用）
- [ ] 配置热重载（当前需重启加载）
- [ ] 更完善的错误处理（自定义错误类型层级）

## 关键文件位置

```
Pstep/
├── Cargo.toml                    # workspace 根
├── pstep-core/
│   ├── Cargo.toml                # v0.3.0, edition 2024
│   └── src/
│       ├── lib.rs                # pub mod config, client, fallback, stats, manager
│       ├── config.rs             # 配置解析 + 13 ✅
│       ├── client.rs             # 模型调用 + 流式 + 8 ✅
│       ├── fallback.rs           # 非流式 + 流式 fallback chain + 6 ✅
│       ├── stats.rs              # SQLite 统计 + 6 ✅
│       └── manager.rs            # ModelStore + 8 ✅
├── pstep-gateway/
│   ├── Cargo.toml                # v0.3.0
│   └── src/main.rs               # 所有路由 + ACP/WS（单文件约 1065 行）
├── config/
│   ├── models.json               # 模型配置（mimo-v2.5）
│   └── pi/                       # Pi 智能体配置（保留）
├── code/                         # 现有 Node.js 代码（保留，过渡期可用）
├── docs/
│   ├── rust-rewrite-design.md    # Rust 重构设计方案
│   ├── kanban.md                 # 任务看板（含详细进度 ✅）
│   ├── claudecode-integration.md # Claude Code 对接指南
│   ├── work-summary.md           # 本文件
│   └── 部署手册.md                # 部署说明
└── CLAUDE.md                     # 项目说明
```

## 已知问题
- Anthropic 端点 `max_tokens` 字段已接收但未传给上游（使用上游默认值）
- Rust 2024 edition 中 `env::set_var`/`remove_var` 是 unsafe（测试中已用 unsafe 块处理）
- `StreamSseChunk.usage` 字段未使用（待实现从 SSE 流提取 usage 功能）
- 所有网关路由写在一个 `main.rs` 文件中（约 1065 行），后续可拆分到 `routes.rs`、`handler.rs`、`acp.rs`
