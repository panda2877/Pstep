# 工作总结 — 2026-05-21

## 项目状态

Pstep（版本 0.3.0）是一个 Rust 实现的 AI 模型网关，采用 workspace 分层架构（pstep-core lib + pstep-gateway bin）。共 51 个单元测试全部通过，Claude Code 端到端集成验证通过。

## 整体架构

```
Client (Claude Code / OpenAI 兼容客户端)
        │
        ├── POST /v1/chat/completions   (OpenAI 格式)
        ├── POST /v1/messages           (Anthropic Messages API 格式)
        ├── POST /anthropic/v1/messages (Anthropic 兼容端点，Claude Code 专用)
        └── WebSocket :3400             (ACP JSON-RPC 协议)
        │
pstep-gateway  (Axum HTTP Server + tokio-tungstenite WS Server)
│   ├── main.rs           # 入口 + AppState + 配置加载
│   ├── routes/
│   │   ├── mod.rs        # Router 构建 + health + stats
│   │   ├── chat.rs       # POST /v1/chat/completions
│   │   ├── anthropic.rs  # POST /v1/messages + Anthropic 格式转换
│   │   └── models.rs     # GET/POST/DELETE /v1/models
│   └── ws.rs             # WebSocket ACP + JSON-RPC
        │
pstep-core     (lib crate)
│   ├── config.rs   — GatewayConfig/ModelConfig 解析，支持从环境变量加载
│   ├── client.rs   — ModelClient (reqwest) 非流式 + 流式 SSE
│   ├── fallback.rs — Fallback chain 引擎（非流式 + 流式）
│   ├── stats.rs    — SQLite token 用量统计
│   └── manager.rs  — ModelStore (RwLock+HashMap) 运行时 CRUD
        │
    外部 LLM API (OpenAI / Anthropic / DeepSeek 等)
```

## 已完成的核心功能

### 项目骨架（全部完成）
- `pstep-core`（lib crate）和 `pstep-gateway`（bin crate）workspace 结构
- 依赖：tokio, axum, reqwest, serde, rusqlite, tracing, mockito 等
- `cargo build` 零 warning 通过；版本 0.3.0，edition 2024

### pstep-core 核心层（全部完成）
| 模块 | 功能描述 | 测试数 |
|------|---------|--------|
| **config.rs** | GatewayConfig/ModelConfig serde 解析，apiKeyEnv→环境变量映射，`.env` 文件加载，`from_json()` 支持从环境变量加载 | 13 ✅ |
| **client.rs** | ModelClient：`call_model()` 非流式 + `call_model_stream()` 流式 SSE，StreamHandle/StreamChunk 类型，自动 Bearer 认证，`reasoning_content` + `PromptTokensDetails` 支持 | 8 ✅ |
| **fallback.rs** | `handle_with_fallback()` 非流式 + `handle_stream_with_fallback()` 流式 fallback chain，响应 model 字段替换为实际模型名 | 6 ✅ |
| **stats.rs** | SQLite stats.db 自动建表（token_usage），`log_usage()` 写入，`recent(N)` 查询 | 6 ✅ |
| **manager.rs** | `ModelProvider` trait 抽象，`ModelStore`（RwLock+HashMap）运行时 CRUD，OpenAIProvider 适配，健康检查 | 8 ✅ |

测试合计：**51 个单元测试全部通过** ✅

### pstep-gateway 网关层（全部完成）

**模块拆分（main.rs 1080 行 → 6 个模块）：**

| 文件 | 内容 | 行数 |
|------|------|------|
| `main.rs` | 入口 + AppState + 配置加载（env 优先，file fallback） | ~108 |
| `routes/mod.rs` | Router 构建 + health + stats | ~50 |
| `routes/chat.rs` | POST /v1/chat/completions + OpenAI SSE | ~100 |
| `routes/anthropic.rs` | POST /v1/messages + Anthropic SSE + 格式转换 | ~450 |
| `routes/models.rs` | GET/POST/DELETE /v1/models | ~90 |
| `ws.rs` | WebSocket ACP + JSON-RPC | ~170 |

**HTTP 路由（端口 3000）：**
| 方法 | 路径 | 说明 |
|------|------|------|
| POST | `/v1/chat/completions` | OpenAI 兼容聊天补全（非流式） |
| POST | `/v1/messages` | Anthropic Messages API 格式兼容 |
| POST | `/anthropic/v1/messages` | Anthropic 兼容端点（Claude Code 专用） |
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

**Anthropic Messages API 兼容：**
- Anthropic content blocks ↔ OpenAI message 双向格式转换
- system prompt 传递（转为 OpenAI system message）
- `tool_use` / `tool_result` 消息格式转换（含 `tool_call_id` 正确传递）
- `thinking` 块 ↔ `reasoning_content` 双向转换
- 非流式：返回 Anthropic 格式 JSON（content[], stop_reason, usage）
- `cache_read_input_tokens` 从 `prompt_tokens_details.cached_tokens` 映射
- id 格式：使用上游原始 id（无 `msg_` 前缀）

**配置迁移（CNB 密钥仓库）：**
- `config/models.json` 从 git 移除（加入 .gitignore）
- 新增 `from_json()` 方法支持从 `PSTEP_CONFIG_JSON` 环境变量加载
- `.cnb.yml` import 密钥并注入环境变量
- 本地开发 fallback 到 `config/models.json` 文件

### 集成验证
- Claude Code 通过 `/anthropic/v1/messages` 端点成功对接
- 多轮对话 + tool_use 场景验证通过
- thinking 块、tool_call_id、cache_read_input_tokens 格式均正确

## 近期修复记录

### 2026-05-20：环境修复 + 代码修复
- 升级 Rust 工具链 1.75.0 → 1.95.0（支持 edition 2024）
- 配置 Cargo 镜像源（rsproxy.cn sparse index）
- 安装 OpenSSL 开发库
- 修复 SSE 解析器：重写为基于 buffer 的逐行解析，支持多消息单 chunk
- 修复 tokio runtime 冲突：使用 `Server::new_async().await`

### 2026-05-21 上午：Serde rename + Anthropic 端点
- 修复 `ModelConfig.api_key` 缺少 `#[serde(rename = "apiKey")]` 导致 401 的问题
- 修复 `ServerConfig.ws_port` 缺少 `#[serde(rename = "wsPort")]` 的问题
- 完成 `POST /v1/messages` 和 Anthropic ↔ OpenAI 格式转换
- 端到端测试（非流式 + 流式）全部通过

### 2026-05-21 下午：模块拆分 + Claude Code 集成
- 拆分 main.rs（1080 行）为 6 个独立模块
- 新增 `/anthropic/v1/messages` 端点（Claude Code 专用）
- 修复 `tool_call_id` 未传递导致 upstream 400 的问题
- 修复 thinking 块丢失导致 "reasoning_content must be passed back" 错误
- 添加 `reasoning_content` 字段支持 thinking 模式
- 添加 `PromptTokensDetails` 结构体支持 `cache_read_input_tokens`
- 配置迁移到 CNB 密钥仓库（`PSTEP_CONFIG_JSON` 环境变量）
- 强制非流式模式（Claude Code 不支持流式）
- Claude Code 端到端集成验证通过

### 2026-05-21 晚上：HTTP 优雅重启
- 实现 HTTP 优雅重启（graceful shutdown）
- 修改 `main.rs` 关闭流程：不再直接 abort HTTP 服务器，改为 await graceful shutdown 完成
- 保证 in-flight 请求在重启时能正常完成，避免 Claude Code 连接中断
- WebSocket 部分保持 abort（当前无 ACP 场景，未来需要时再实现优雅关闭）
- 编译验证通过

## 待完成项

### 验证与测试
- [ ] 多模型 fallback 场景验证（至少 2 个模型，含主模型故障切换）
- [ ] 验证 `/stats` 中 `model` vs `requested_model` 记录正确
- [ ] ACP WebSocket 连接测试
- [ ] pstep-gateway 单元测试（路由处理、ACP 协议解析）
- [ ] `/v1/messages` 端点的 `max_tokens` 字段已接收但未传给上游（使用上游默认值）

### 生产化
- [x] HTTP 优雅重启（graceful shutdown，等待 in-flight 请求完成）
- [ ] 从 SSE 流提取 usage 信息（`StreamSseChunk.usage` 字段未使用）
- [ ] 配置热重载（当前需重启加载）
- [ ] 更完善的错误处理（自定义错误类型层级）
- [ ] CNB 密钥仓库创建 `pstep-config.yml`（需用户手动操作）

## 关键文件位置

```
Pstep/
├── Cargo.toml                    # workspace 根
├── .cnb.yml                      # CI 配置 + 密钥导入
├── .gitignore                    # 含 config/models.json
├── pstep-core/
│   ├── Cargo.toml                # v0.3.0, edition 2024
│   └── src/
│       ├── lib.rs                # pub mod config, client, fallback, stats, manager
│       ├── config.rs             # 配置解析 + from_json() + 13 ✅
│       ├── client.rs             # 模型调用 + reasoning_content + 8 ✅
│       ├── fallback.rs           # 非流式 + 流式 fallback chain + 6 ✅
│       ├── stats.rs              # SQLite 统计 + 6 ✅
│       └── manager.rs            # ModelStore + 8 ✅
├── pstep-gateway/
│   ├── Cargo.toml                # v0.3.0
│   └── src/
│       ├── main.rs               # 入口 + AppState + 配置加载 (~108 行)
│       ├── routes/
│       │   ├── mod.rs            # Router 构建 + health + stats (~50 行)
│       │   ├── chat.rs           # OpenAI chat completions (~100 行)
│       │   ├── anthropic.rs      # Anthropic messages + 格式转换 (~450 行)
│       │   └── models.rs         # Model CRUD (~90 行)
│       └── ws.rs                 # WebSocket ACP + JSON-RPC (~170 行)
├── config/
│   └── models.json               # 模型配置（gitignored，本地开发用）
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
- CNB 密钥仓库 `pstep-config.yml` 需用户手动创建
- WebSocket ACP 服务器重启时会强制断开连接（未来实现 ACP 场景时需改为优雅关闭）
