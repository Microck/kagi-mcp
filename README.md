# kagi-mcp

`kagi-mcp` is a tiny MCP server built on top of [`kagi-cli`](https://github.com/Microck/kagi-cli).

It is intentionally just an extra repo:

- separate repo
- one Rust binary
- wraps the `kagi` CLI instead of reimplementing Kagi logic
- returns the same JSON the CLI already emits

## Requirements

- A working `kagi` binary on `PATH`, or `KAGI_CLI_PATH` pointing to it
- Kagi credentials provided through environment variables
  - `KAGI_SESSION_TOKEN`
  - `KAGI_API_TOKEN`

`.kagi.toml` is not the recommended auth path for MCP usage because the CLI resolves it relative to the server process working directory.

## Build

```bash
cargo build --release
```

## Run

```bash
KAGI_CLI_PATH=/path/to/kagi \
KAGI_SESSION_TOKEN=... \
KAGI_API_TOKEN=... \
./target/release/kagi-mcp
```

Optional:

- `KAGI_CLI_PATH`: explicit path to the `kagi` binary
- `KAGI_MCP_TIMEOUT_MS`: subprocess timeout in milliseconds, default `30000`

## Claude Desktop

```json
{
  "mcpServers": {
    "kagi": {
      "command": "/home/ubuntu/workspace/kagi-mcp/target/release/kagi-mcp",
      "env": {
        "KAGI_CLI_PATH": "/home/ubuntu/.nvm/versions/node/v24.13.0/bin/kagi",
        "KAGI_SESSION_TOKEN": "your-session-token",
        "KAGI_API_TOKEN": "your-api-token"
      }
    }
  }
}
```

## Tools

- `kagi_search`
- `kagi_summarize`
- `kagi_news`
- `kagi_news_categories`
- `kagi_news_chaos`
- `kagi_assistant`
- `kagi_fastgpt`
- `kagi_enrich_web`
- `kagi_enrich_news`
- `kagi_smallweb`
- `kagi_auth_status`
- `kagi_auth_check`

## Test

```bash
cargo test
```
