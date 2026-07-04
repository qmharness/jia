# JIA · Quick Start

## Prerequisites

- Rust toolchain 1.85+ (edition 2024)
- SQLite 3

## Build

```bash
cargo build --release
```

Binary at `target/release/jia`.

## Configuration

Choose one:

```bash
# Option 1: config file
cp config.example.toml config.toml
```

```bash
# Option 2: env var for config path
export JIA_CONFIG="/path/to/config.toml"
```

API keys and other secrets go in `config.toml`.

## Run

```bash
jia                  # Launch TUI (terminal interface)
jia gateway start    # Start HTTP/SSE gateway (background)
jia gateway stop     # Stop gateway
jia gateway status   # Show gateway status
jia doctor           # Diagnostic health check
jia tui              # Launch TUI explicitly (same as bare `jia`)
```

## Docker

```bash
docker build -t jia .
docker run -p 3000:3000 -v ./config.toml:/data/config.toml jia
```

## IM Bots

Configure in `config.toml`:

```toml
[bots.wechat]
enabled = true

[bots.telegram]
enabled = true
token = "your-bot-token"
```

## MCP Tool Extension

Declare MCP servers in `config.toml`. Tools are auto-discovered on startup:

```toml
[[mcp_server]]
name = "filesystem"
command = "npx"
args = ["-y", "@modelcontextprotocol/server-filesystem", "/tmp"]
```
