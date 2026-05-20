# 工作总结 — 2026-05-20

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
- **client.rs** — ModelClient，call_model() 非流式 + call_model_stream() 流式 SSE，StreamHandle/StreamChunk 类型，8 个 mockito 单元测试已编写（待验证）
- **fallback.rs** — handle_with_fallback() 非流式 fallback chain，响应 model 字段替换为实际模型名
- **stats.rs** — SQLite stats.db，token_usage 表自动建表，log_usage() + recent() 查询
- **manager.rs** — ModelStore（RwLock + HashMap），运行时 CRUD

### 3. pstep-gateway 网关层（基础完成）
- main.rs：Axum HTTP 服务器（端口 3000），tracing 日志
- POST /v1/chat/completions → 调用 core fallback → 返回结果
- GET /health → `{ status: "ok" }`
- GET /stats → 最近 10 条用量记录

## 中断点

**网络问题**：WSL2 环境无法访问 crates.io（ping 100% 丢包），导致新加的 `mockito` 依赖无法下载。

## 下一步需要做的事

### 立即（验证已写代码）
1. **恢复网络后跑测试**：`cargo test -p pstep-core` — 验证 client 流式测试和 config 测试全部通过
2. 如果 mockito 下载不了，可以换成 `wiremock` 或手写一个简单的 test HTTP server

### 接着做（按 kanban 优先级）
3. **pstep-core 单元测试**：fallback、stats、model manager 的测试
4. **流式 fallback**：fallback.rs 里的流式分支（目前只实现了非流式）
5. **Gateway 流式 SSE 透传**：main.rs 的 /v1/chat/completions 支持 stream=true
6. **WebSocket / ACP**：端口 3400 的 ACP JSON-RPC 协议处理
7. **模型管理接口**：GET/POST/DELETE /v1/models
8. **优雅关闭**：SIGINT 处理

## 关键文件位置

```
Pstep/
├── Cargo.toml                    # workspace 根
├── pstep-core/
│   ├── Cargo.toml                # 依赖定义（含 dev-dependencies: tempfile, mockito）
│   └── src/
│       ├── lib.rs                # pub mod config, client, fallback, stats, manager
│       ├── config.rs             # 配置解析 + 13 个测试 ✅
│       ├── client.rs             # 模型调用 + 流式 + 8 个测试（待验证）
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
- Rust 2024 edition 中 `env::set_var`/`remove_var` 是 unsafe（测试中已用 unsafe 块处理）
