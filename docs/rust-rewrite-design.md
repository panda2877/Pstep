# Pstep Rust 重构设计方案

## 一、背景与目标

当前 Pstep 是一个基于 Node.js (Express) 的 AI 模型网关，集成了 Pi 编码智能体核心和 ACP (Agent Client Protocol) 协议。现有实现存在以下问题：

- Node.js 单线程模型，高并发下性能受限
- 流式代理的内存和延迟表现不佳
- 缺少类型安全，配置解析容易出错
- 网关与模型层耦合，不利于模型管理功能扩展

**重构目标：**

1. 用 Rust 重写，采用 workspace 分层架构（core / gateway 分离）
2. 保持 OpenAI 兼容 API，确保 Claude Code 等第三方 Agent 可直接对接
3. 保留 ACP WebSocket 协议支持
4. 保留 fallback 链、token 用量统计等核心能力
5. 保持 `config/pi/` 目录结构，兼容现有 Pi 智能体配置
6. 支持多端交互层（VS Code 插件、Web 前端、移动端）

## 二、整体架构

```
┌──────────────────────────────────────────────────────────┐
│                    交互层 (Clients)                        │
│                                                          │
│   ┌──────────┐  ┌──────────┐  ┌──────────┐             │
│   │ VS Code  │  │   Web    │  │  Mobile  │             │
│   │ Extension│  │ Frontend │  │   App    │             │
│   └────┬─────┘  └────┬─────┘  └────┬─────┘             │
│        │              │              │                    │
└────────┼──────────────┼──────────────┼──────────────────┘
         │              │              │
         └──────────────┼──────────────┘
                        │
              OpenAI API + ACP (WebSocket)
                        │
┌───────────────────────┼──────────────────────────────────┐
│                 网关层 (pstep-gateway)                     │
│                       │                                   │
│  ┌───────────┐  ┌─────┴─────┐  ┌──────────┐            │
│  │ HTTP :3000│  │  Router   │  │WS :3400  │            │
│  │ (Axum)    │  │           │  │ (ACP)    │            │
│  └─────┬─────┘  └─────┬─────┘  └────┬─────┘            │
│        │              │              │                    │
└────────┼──────────────┼──────────────┼──────────────────┘
         │              │              │
         └──────────────┼──────────────┘
                        │
┌───────────────────────┼──────────────────────────────────┐
│                   核心层 (pstep-core)                      │
│                       │                                   │
│  ┌───────────┐  ┌─────┴─────┐  ┌──────────┐            │
│  │  Model    │  │ Fallback  │  │  Stats   │            │
│  │  Manager  │  │  Engine   │  │ (SQLite) │            │
│  └─────┬─────┘  └─────┬─────┘  └──────────┘            │
│        │              │                                   │
│  ┌─────┴──────────────┴──────────────────┐              │
│  │         Model Client (reqwest)         │              │
│  └───────────────────┬───────────────────┘              │
└──────────────────────┼───────────────────────────────────┘
                       │
                 ┌─────┴─────┐
                 │  LLM APIs  │
                 │ OpenAI     │
                 │ Anthropic  │
                 │ DeepSeek   │
                 │ ...        │
                 └───────────┘
```

**请求链路：** Client → Gateway → Core → 外部 LLM API

外部 Agent（如 Claude Code）只需对接网关的 OpenAI 兼容 API，无需感知 core 层的存在。

## 三、技术选型

### Rust 层

| 模块 | 选型 | 理由 |
|------|------|------|
| HTTP 框架 | **Axum** | 异步、tower 中间件生态、与 tokio 深度集成 |
| WebSocket | **axum::extract::ws** | 与 Axum 无缝配合 |
| HTTP 客户端 | **reqwest** (streaming) | 原生支持 SSE 流式代理 |
| 异步运行时 | **tokio** | Rust 异步事实标准 |
| 数据库 | **rusqlite** | 轻量，通过 `tokio::task::spawn_blocking` 桥接 |
| 配置解析 | **serde + serde_json** | 类型安全的 JSON 配置读取 |
| 日志 | **tracing** | 结构化日志，支持 span 追踪 |
| CLI 参数 | **clap** | 启动参数解析 |

### 交互层

| 端 | 技术 | 与网关交互方式 |
|---|------|--------------|
| VS Code 插件 | TypeScript + VS Code Extension API | HTTP (聊天补全) + WebSocket (ACP) |
| Web 前端 | React / Vue | HTTP (聊天补全) + WebSocket (ACP) |
| 移动端 | React Native / Flutter | HTTP (聊天补全) + WebSocket (ACP) |

交互层各应用独立构建、独立部署，网关是它们唯一的后端入口。

## 四、Workspace 结构

```
Pstep/
├── Cargo.toml                  # workspace 根
├── pstep-core/                 # 核心层 (lib crate)
│   ├── Cargo.toml
│   └── src/
│       ├── lib.rs
│       ├── config.rs           # 模型配置解析
│       ├── client.rs           # reqwest 调用远程模型
│       ├── stream.rs           # SSE 流式代理
│       ├── fallback.rs         # Fallback chain 引擎
│       ├── stats.rs            # SQLite 用量统计
│       └── manager/
│           ├── mod.rs
│           ├── store.rs        # 模型配置持久化
│           ├── provider.rs     # 多 provider 抽象 trait
│           └── health.rs       # 模型健康检查
├── pstep-gateway/              # 网关层 (bin crate)
│   ├── Cargo.toml
│   └── src/
│       ├── main.rs             # 入口：解析配置、启动 HTTP + WS
│       ├── routes.rs           # Axum 路由定义
│       ├── handler.rs          # /v1/chat/completions 处理逻辑
│       └── acp.rs              # ACP WebSocket 协议处理
├── pstep-vscode/               # VS Code 插件 (TypeScript)
├── pstep-web/                  # Web 前端 (React/Vue)
├── pstep-mobile/               # 移动端 (React Native/Flutter)
├── config/
│   ├── models.json             # 模型配置
│   └── pi/                     # Pi 智能体配置
│       ├── settings.json
│       ├── models.json
│       └── auth.json
├── docs/
└── CLAUDE.md
```

**编译产物：** `cargo build --release` 在 workspace 根执行，输出单个可执行文件 `target/release/pstep-gateway`。`pstep-core` 作为 lib crate 编译进二进制，无额外产物。

## 五、模块设计

### 5.1 核心层 — pstep-core

#### 5.1.1 配置模块

```rust
pub struct GatewayConfig {
    pub models: HashMap<String, ModelConfig>,
    pub fallback_chains: HashMap<String, Vec<String>>,
    pub server: ServerConfig,
}

pub struct ModelConfig {
    pub url: String,
    pub api_key: String,           // 从环境变量或 apiKeyEnv 解析
    pub remote_model: Option<String>,
}

pub struct ServerConfig {
    pub port: u16,                 // default: 3000
    pub ws_port: u16,              // default: 3400
}
```

- 启动时从 `config/models.json` 读取，`apiKeyEnv` 字段映射到环境变量
- 支持 `.env` 文件（通过 `dotenvy` crate）

#### 5.1.2 模型调用模块

```rust
pub async fn call_model(config: &ModelConfig, body: &ChatRequest) -> Result<ModelResponse>;
pub async fn call_model_stream(config: &ModelConfig, body: &ChatRequest) -> Result<StreamResponse>;
```

- `call_model`: 非流式，等待完整响应
- `call_model_stream`: 流式，返回 `mpsc::Receiver<Bytes>` 供上层 pipe 到客户端
- 请求超时默认 30s，可配置
- 统一处理 `Authorization: Bearer <key>` 认证头

#### 5.1.3 Fallback 引擎

```rust
pub async fn handle_with_fallback(
    config: &GatewayConfig,
    request: &ChatRequest,
) -> Result<FallbackResult>;
```

- 非流式：遍历 fallback chain，依次调用，首个成功即返回
- 流式：仅使用 chain 中第一个模型（流式无法重试），失败则报错
- **响应中的 `model` 字段替换为实际使用的模型名**，而非请求中的原始 model 值
- 每次调用均触发 `log_usage` 记录

`FallbackResult` 结构：

```rust
pub struct FallbackResult {
    pub data: ChatCompletionResponse,   // OpenAI 兼容响应体
    pub actual_model: String,           // 实际使用的模型名
    pub requested_model: String,        // 客户端请求的 model 名
}
```

#### 5.1.4 模型管理模块

模型管理是核心领域逻辑，不属于网关协议层。

```rust
// 模型 Provider 抽象 trait — 后续扩展多 provider 的基础
#[async_trait]
pub trait ModelProvider: Send + Sync {
    async fn chat(&self, request: &ChatRequest) -> Result<ChatCompletionResponse>;
    async fn chat_stream(&self, request: &ChatRequest) -> Result<StreamResponse>;
    fn health_check(&self) -> bool;
    fn name(&self) -> &str;
}

// 模型存储 — 管理模型配置的增删改查
pub struct ModelStore {
    models: RwLock<HashMap<String, ModelEntry>>,
}

pub struct ModelEntry {
    pub name: String,
    pub provider: Box<dyn ModelProvider>,
    pub config: ModelConfig,
    pub fallback_chain: Vec<String>,
}
```

**管理能力：**

| 功能 | 说明 |
|------|------|
| 模型 CRUD | 运行时增删改查模型配置 |
| API Key 管理 | 与模型绑定，支持热更新 |
| Fallback 配置 | 运行时调整 fallback chain |
| Provider 适配 | OpenAI / Anthropic / DeepSeek 等，通过 trait 抽象 |
| 健康检查 | 定期探测模型可用性 |

#### 5.1.5 统计模块

SQLite 表结构：

```sql
CREATE TABLE IF NOT EXISTS token_usage (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    timestamp DATETIME DEFAULT CURRENT_TIMESTAMP,
    model TEXT,                  -- 实际使用的模型
    requested_model TEXT,        -- 客户端请求的模型
    success INTEGER,
    input_tokens INTEGER,
    output_tokens INTEGER,
    total_tokens INTEGER,
    latency_ms INTEGER,
    error TEXT
);
```

- 写入通过 `tokio::task::spawn_blocking` 桥接 rusqlite 同步调用
- `model` 记录真实调用的模型，`requested_model` 记录客户端原始请求，便于排查 fallback 行为和成本核算

### 5.2 网关层 — pstep-gateway

网关层是薄的协议适配层，只负责 HTTP/WS 路由和协议转换，不含业务逻辑。

#### 5.2.1 HTTP 路由

| 方法 | 路径 | 说明 |
|------|------|------|
| POST | `/v1/chat/completions` | OpenAI 兼容聊天补全 |
| GET | `/health` | 健康检查 |
| GET | `/stats` | 最近 token 用量统计 |
| POST | `/v1/models` | 模型管理（增/改） |
| DELETE | `/v1/models/:name` | 删除模型 |
| GET | `/v1/models` | 列出已注册模型 |

`/v1/chat/completions` 处理流程：

```
请求进入 → 解析 model 字段 → 查找 fallback chain
→ 依次尝试 chain 中的模型 → 返回第一个成功结果
→ 响应中 model 字段替换为实际使用的模型名
→ 记录用量到 SQLite
```

**流式处理：** 对 `stream: true` 的请求，直接将上游 SSE 事件透传给客户端，结束时从最后的 chunk 提取 usage 统计。

#### 5.2.2 WebSocket / ACP

在独立端口（默认 3400）运行 WebSocket 服务，处理 ACP JSON-RPC 协议：

- `initialize` → 返回 capabilities
- `prompt` → 提取用户消息，调用 core 层模型，返回结果
- 其他方法 → 返回 `-32601 Method not found`

## 六、与现有 Node.js 的差异

| 特性 | Node.js | Rust |
|------|---------|------|
| 架构 | 单体 | workspace 分层 (core + gateway) |
| 并发模型 | 单线程事件循环 | 多线程异步 (tokio) |
| 流式代理 | axios stream + pipe | reqwest stream + axum streaming |
| 内存占用 | 较高（V8 堆） | 较低（无 GC 停顿） |
| 类型安全 | 无（纯 JS） | 编译期保证 |
| 模型管理 | 静态配置 | 核心层支持运行时 CRUD + 多 provider |
| 响应 model 字段 | 透传上游值 | 替换为实际使用的模型名 |
| 二进制分发 | 需 Node.js 运行时 | 单个可执行文件 |

## 七、与 Claude Code 等 Agent 的集成

Claude Code 通过 OpenAI 兼容 API 调用模型网关。集成方式：

1. Pstep 网关监听在 Claude Code 可访问的地址（默认 `localhost:3000`）
2. Claude Code 配置中将 API base URL 指向 `http://localhost:3000/v1`
3. 请求中的 `model` 字段对应 `config/models.json` 中的 fallback chain 名称

Claude Code 的 `settings.json` 配置示例：

```json
{
  "env": {
    "ANTHROPIC_BASE_URL": "http://localhost:3000/v1",
    "ANTHROPIC_AUTH_TOKEN": "any-value"
  }
}
```

**响应中的 model 字段：** 当 Pstep 配置了多个模型时，响应会返回实际使用的模型名称，而非客户端请求的 model 值。例如：

```json
// 请求: { "model": "gpt-4" }
// Fallback chain 实际用了 "deepseek-chat"
// 响应:
{
  "id": "chatcmpl-xxx",
  "model": "deepseek-chat",
  "choices": [...],
  "usage": { "prompt_tokens": 10, "completion_tokens": 20, "total_tokens": 30 }
}
```

这让客户端能准确知道真实消耗的是哪个模型，便于排查问题和成本核算。

## 八、部署

### 构建

```bash
# 开发构建
cargo build

# Release 构建（单个二进制文件）
cargo build --release

# 交叉编译 Linux musl（静态链接，无系统依赖）
cargo build --release --target x86_64-unknown-linux-musl
```

### 部署

```bash
# 单文件部署
scp target/release/pstep-gateway deployer@server:~/Pstep/

# 运行
cd ~/Pstep && ./pstep-gateway
```

无需 Node.js 运行时，单个二进制文件即可运行。

### 配置

- `config/models.json` — 模型配置（复用现有格式）
- `config/pi/` — Pi 智能体配置（复用现有）
- `.env` — 环境变量（API Key 等）

## 九、实施计划

### Phase 1：基础框架
- 初始化 Cargo workspace（pstep-core + pstep-gateway）
- 实现 core 配置模块（serde 解析 models.json）
- 实现 gateway `/health` 端点

### Phase 2：核心代理
- 实现 core 模型调用客户端 (reqwest)
- 实现 fallback chain 引擎
- 实现 gateway `/v1/chat/completions` 非流式代理
- **实现响应 model 字段替换为实际使用的模型名**

### Phase 3：流式支持
- 实现 SSE 流式代理透传
- 流式 usage 统计提取

### Phase 4：WebSocket / ACP
- 实现 ACP WebSocket 服务器
- 处理 initialize / prompt 协议

### Phase 5：统计与模型管理
- 集成 SQLite 统计模块
- 实现 core 模型管理（CRUD、Provider trait）
- 实现 gateway 管理接口

### Phase 6：交互层
- VS Code 插件原型
- Web 前端原型
- 移动端原型

### Phase 7：生产化
- 错误处理完善（自定义错误类型）
- 结构化日志 (tracing)
- 配置热重载
- 单元测试与集成测试
