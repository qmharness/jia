# JIA · Quick Start

## Prerequisites

- **Rust** 1.85+ (edition 2024)

  macOS / Linux:
  ```bash
  curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh
  ```

  Windows (PowerShell): download and run [rustup-init.exe](https://rustup.rs).  
  Follow the prompts — the installer will offer to install Visual Studio Build Tools if needed.

  Restart your terminal after installation, then verify:
  ```bash
  rustc --version   # ≥ 1.85.0
  cargo --version
  ```

- **SQLite**: bundled — no separate install needed.

## Install

```bash
cargo install --git https://github.com/qmharness/jia.git
```

Or build from source:

```bash
cargo build --release
# binary at target/release/jia
```

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
jia web              # Start gateway with web dashboard
jia init             # Initialize a Jia project in current directory
jia start            # Start gateway (shortcut)
jia stop             # Stop gateway (shortcut)
jia restart          # Restart gateway (shortcut)
jia status           # Show gateway status (shortcut)
jia doctor           # Diagnostic health check
jia gateway start    # Start gateway (full command)
jia gateway stop     # Stop gateway (full command)
```

## Docker

```bash
docker build -t jia .
docker run -p 3000:3000 -v ./config.toml:/data/config.toml jia
```

## IM Bots

```bash
jia wechat-setup     # QR login for WeChat personal bot
```

Configure in `config.toml`:

```toml
[bots.telegram]
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
