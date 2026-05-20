# 里程碑：Rust 重构 + 外部 Agent 对接 (Claude Code)

## 一、核心层 pstep-core

### 1.1 项目初始化
- [x] 初始化 Cargo workspace（根 Cargo.toml）
- [x] 创建 pstep-core lib crate
- [x] 创建 pstep-gateway bin crate
- [x] 引入基础依赖（tokio, axum, reqwest, serde, rusqlite, tracing）
- [x] 确认 `cargo build` 通过

### 1.2 配置模块
- [x] 实现 `GatewayConfig` 结构体（models, fallback_chains, server）
- [x] 实现 `ModelConfig` 结构体（url, api_key, remote_model）
- [x] 实现 `config/models.json` 读取与解析
- [x] 实现 `apiKeyEnv` → 环境变量映射
- [x] 支持 `.env` 文件加载（dotenvy）
- [x] 单元测试：配置解析、环境变量缺失时的错误处理

### 1.3 模型调用客户端
- [x] 实现 `call_model()` 非流式调用（reqwest POST）
- [x] 实现 `call_model_stream()` 流式调用（reqwest SSE）
- [x] 统一 Authorization Bearer 认证头注入
- [x] 请求超时配置（默认 30s）
- [x] 错误处理：网络错误、超时、上游 4xx/5xx
- [x] 单元测试：mock reqwest client 验证请求构造（待验证，见工作总结）

### 1.4 Fallback 引擎
- [x] 实现 `handle_with_fallback()` 非流式逻辑（遍历 chain，首个成功即返回）
- [ ] 实现流式 fallback（仅用 chain 第一个模型，失败报错）
- [x] FallbackResult 返回 `actual_model` + `requested_model`
- [x] 响应体 `model` 字段替换为实际使用的模型名
- [ ] 单元测试：多模型 fallback、全部失败、链为空

### 1.5 统计模块
- [x] 实现 SQLite 初始化（`token_usage` 表自动建表）
- [x] 实现 `log_usage()` 写入（spawn_blocking 桥接）
- [x] 实现查询接口（最近 N 条记录）
- [ ] 单元测试：写入→查询一致性

### 1.6 模型管理模块
- [ ] 定义 `ModelProvider` trait（chat, chat_stream, health_check, name）
- [x] 实现 `ModelStore`（RwLock + HashMap，支持运行时 CRUD）
- [ ] 实现 OpenAI provider 适配（作为首个 provider 实现）
- [ ] 实现模型健康检查（定期探测 /health 或轻量请求）
- [ ] 单元测试：ModelStore 增删改查

## 二、网关层 pstep-gateway

### 2.1 基础路由
- [x] 实现 Axum HTTP 服务器启动（端口 3000）
- [x] 实现 `POST /v1/chat/completions`（调用 core fallback → 返回结果）
- [x] 实现 `GET /health`（返回 `{ status: "ok" }`）
- [x] 实现 `GET /stats`（返回最近 10 条用量记录）
- [ ] 流式响应处理（SSE 透传，`text/event-stream`）

### 2.2 模型管理接口
- [ ] 实现 `GET /v1/models`（列出已注册模型）
- [ ] 实现 `POST /v1/models`（注册/更新模型）
- [ ] 实现 `DELETE /v1/models/:name`（删除模型）

### 2.3 WebSocket / ACP
- [ ] 实现 WebSocket 服务器（端口 3400）
- [ ] 处理 `initialize` JSON-RPC → 返回 capabilities
- [ ] 处理 `prompt` JSON-RPC → 提取消息 → 调用 core → 返回结果
- [ ] 处理未知方法 → 返回 `-32601 Method not found`
- [ ] 连接管理（日志记录连接/断开）

### 2.4 入口与启动
- [x] 实现 `main.rs`：解析 CLI 参数 → 加载配置 → 启动 HTTP + WS
- [ ] 优雅关闭（SIGINT 处理）
- [x] 结构化日志初始化（tracing-subscriber）

## 三、外部 Agent 对接

### 3.1 Claude Code 对接验证
- [ ] 本地启动 pstep-gateway
- [ ] 配置 Claude Code `ANTHROPIC_BASE_URL=http://localhost:3000/v1`
- [ ] 验证 Claude Code 能正常发送聊天补全请求
- [ ] 验证响应中 `model` 字段返回实际使用的模型名
- [ ] 验证流式响应正常工作
- [ ] 记录对接配置文档（`docs/claudecode-integration.md`）

### 3.2 多模型场景验证
- [ ] 配置 fallback chain（至少 2 个模型）
- [ ] 验证主模型可用时使用主模型
- [ ] 验证主模型不可用时自动 fallback 到备选模型
- [ ] 验证 `/stats` 中 `model` vs `requested_model` 记录正确

## 四、测试与质量

### 4.1 单元测试
- [x] pstep-core config：13 个测试全部通过
- [ ] pstep-core client：流式+非流式 mock 测试已编写，待验证（见工作总结）
- [ ] pstep-core：fallback、stats、model manager
- [ ] pstep-gateway：路由处理、ACP 协议解析

### 4.2 集成测试
- [ ] 启动完整网关 → 发送请求 → 验证响应格式
- [ ] 流式请求端到端测试
- [ ] ACP WebSocket 连接测试

### 4.3 文档更新
- [x] CLAUDE.md：已创建（Node.js 版，待补充 Rust 部分）
- [x] `docs/rust-rewrite-design.md`：已创建完整设计方案
- [ ] `docs/claudecode-integration.md`（Claude Code 对接指南）
