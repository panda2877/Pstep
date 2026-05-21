# 工作总结 — 2026-05-21

## 项目状态

Pstep 正在从 Node.js 重构为 Rust，采用 workspace 分层架构（pstep-core lib + pstep-gateway bin）。

## 已完成的工作

### 1. 项目骨架（全部完成）
- Cargo workspace 初始化，pstep-core 和 pstep-gateway 两个 crate
- 所有依赖引入：tokio, axum, reqwest, serde, rusqlite, tracing, mockito 等
- `cargo build` 零 warning 通过
- `config/` 目录从 `code/` 提取到项目根目录，Rust 和 Node.js 共用

### 2. pstep-core 核心层（大部分完成）
- **config.rs** — GatewayConfig/ModelConfig 结构体，serde 解析 models.json，apiKeyEnv→环境变量映射，13 个单元测试全部通过
- **client.rs** — ModelClient，call_model() 非流式 + call_model_stream() 流式 SSE，StreamHandle/StreamChunk 类型，8 个 mockito 单元测试全部通过
- **fallback.rs** — handle_with_fallback() 非流式 fallback chain，响应 model 字段替换为实际模型名
- **stats.rs** — SQLite stats.db，token_usage 表自动建表，log_usage() + recent() 查询
- **manager.rs** — ModelStore（RwLock + HashMap），运行时 CRUD

### 3. pstep-gateway 网关层（基础完成）
- main.rs：Axum HTTP 服务器（端口 3000），tracing 日志
- POST /v1/chat/completions → 调用 core fallback → 返回结果
- GET /health → `{ status: "ok" }`
- GET /stats → 最近 10 条用量记录

## 今日工作（2026-05-20）

### 环境修复
1. **升级 Rust 工具链**：1.75.0 → 1.95.0（支持 edition 2024）
2. **配置 Cargo 镜像源**：rsproxy.cn sparse index
3. **安装 OpenSSL 开发库**：pkg-config, libssl-dev

### 代码修复
4. **修复 client.rs 编译错误**：
   - `Usage` 结构体添加 `Clone` trait
   - `Message.content` 改为 `Option<String>`（SSE delta 中可选）
   - `Message.role` 改为 `Option<String>`（SSE delta 中可选）
5. **修复 SSE 解析器**：重写为基于 buffer 的逐行解析，支持多消息单 chunk
6. **修复 tokio runtime 冲突**：使用 `Server::new_async().await` 替代 `Server::new()`

### 测试结果
- **pstep-core config**：13 个测试 ✅
- **pstep-core client**：8 个测试 ✅
- **总计**：21 个单元测试全部通过

## 今日工作（2026-05-21）

### Bug 修复
1. **修复 apiKey 字段解析**：`ModelConfig.api_key` 缺少 `#[serde(rename = "apiKey")]`，导致配置文件中的 apiKey 无法正确反序列化，网关返回 401。添加 rename 属性后问题解决。

### Anthropic Messages API 兼容
2. **新增 `POST /v1/messages` 端点**：Anthropic Messages API 格式兼容，支持 Claude Code 直连
3. **消息格式转换**：Anthropic content blocks → OpenAI string message
4. **system prompt 传递**：将 Anthropic `system` 参数转为 OpenAI system message
5. **流式 SSE 事件链**：`message_start` → `content_block_start` → `content_block_delta` → `content_block_stop` → `message_delta` → `message_stop`

### 集成验证
6. **非流式聊天补全**：通过 gateway 代理到 mimo-v2.5，返回 HTTP 200 + 正确响应
7. **流式聊天补全**：SSE 流正常透传，包含 data chunks 和 [DONE] 标记
8. **Anthropic 端点测试**：非流式、流式、system prompt 全部正常
9. **测试结果**：42 个单元测试全部通过

## 下一步需要做的事

### 接着做（按 kanban 优先级）
1. **Claude Code 完整对话验证**：配置 Claude Code 使用网关，测试多轮对话
2. **多模型 fallback 场景验证**：配置 fallback chain，验证自动切换
3. **ACP WebSocket 连接测试**：验证端口 3400 的 JSON-RPC 协议
4. **记录对接配置文档**：`docs/claudecode-integration.md`

## 关键文件位置

```
Pstep/
├── Cargo.toml                    # workspace 根
├── pstep-core/
│   ├── Cargo.toml                # 依赖定义（含 dev-dependencies: tempfile, mockito）
│   └── src/
│       ├── lib.rs                # pub mod config, client, fallback, stats, manager
│       ├── config.rs             # 配置解析 + 13 个测试 ✅
│       ├── client.rs             # 模型调用 + 流式 + 8 个测试 ✅
│       ├── fallback.rs           # Fallback chain（非流式完成，流式未实现）
│       ├── stats.rs              # SQLite 统计
│       └── manager.rs            # ModelStore
├── pstep-gateway/
│   └── src/main.rs               # Axum 服务器 + 路由
├── config/
│   ├── models.json               # 模型配置
│   └── pi/                       # Pi 智能体配置
├── code/                         # 现有 Node.js 代码（保留）
├── docs/
│   ├── rust-rewrite-design.md    # Rust 重构设计方案
│   ├── kanban.md                 # 任务看板
│   └── work-summary.md           # 本文件
└── CLAUDE.md                     # 项目说明
```

## 已知问题
- ServerConfig 的 ws_port 字段需要 `#[serde(rename = "wsPort")]` 才能正确解析（已修复）
- ModelConfig 的 api_key 字段需要 `#[serde(rename = "apiKey")]` 才能正确解析（已修复）
- Anthropic 端点 `max_tokens` 字段已接收但未传给上游（使用上游默认值）
- Rust 2024 edition 中 `env::set_var`/`remove_var` 是 unsafe（测试中已用 unsafe 块处理）
- `StreamSseChunk.usage` 字段未使用（待实现从 SSE 流提取 usage 功能）
