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
# 方式二：环境变量
export JIA_ANTHROPIC_API_KEY="sk-..."
export JIA_DEFAULT_PROVIDER="anthropic"
```

配置文件支持四层优先级：环境变量 > `.env` > CLI 参数 > `config.toml`。

## 运行

```bash
jia gateway start    # 启动 HTTP/SSE 网关
jia dashboard        # 打开 Web 界面
jia run "你好，甲"    # 单次推理
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
