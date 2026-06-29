# 甲 · 快速开始

## 环境要求

- Rust 工具链 1.85+（edition 2024）
- SQLite 3

## 构建

```bash
cargo build --release
```

编译产物位于 `target/release/jia`。

## 配置

二选一：

```bash
# 方式一：配置文件
cp config.example.toml config.toml
```

```bash
# 方式二：环境变量指定配置文件路径
export JIA_CONFIG="/path/to/config.toml"
```

API key 等敏感信息在 `config.toml` 中配置。

## 运行

```bash
jia                  # 启动 TUI（终端界面）
jia gateway start    # 后台启动 HTTP/SSE 网关
jia gateway stop     # 停止网关
jia gateway status   # 查看网关状态
jia doctor           # 诊断安装健康
jia tui              # 显式启动 TUI（同 bare jia）
```

## Docker

```bash
docker build -t jia .
docker run -p 8080:8080 -e JIA_ANTHROPIC_API_KEY="sk-..." jia
```

## IM 机器人

甲支持通过 IM 通道交互，需在 `config.toml` 中配置：

```toml
[bots.wechat]
enabled = true

[bots.telegram]
enabled = true
token = "your-bot-token"

[bots.discord]
enabled = true
public_key = "your-ed25519-public-key"
```

## MCP 工具扩展

在 `config.toml` 中声明 MCP server，启动时自动发现并注册工具：

```toml
[[mcp_server]]
name = "filesystem"
command = "npx"
args = ["-y", "@modelcontextprotocol/server-filesystem", "/tmp"]
```
