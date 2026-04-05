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
  - `KAGI_SESSION_TOKEN` — for subscriber features (search, quick, assistant, translate, etc.)
  - `KAGI_API_TOKEN` — for paid API features (summarize, fastgpt, enrich)

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

Optional environment variables:

| Variable | Default | Description |
|----------|---------|-------------|
| `KAGI_CLI_PATH` | `kagi` (on PATH) | Path to the `kagi` binary |
| `KAGI_MCP_TIMEOUT_MS` | `30000` | Subprocess timeout in milliseconds (must be > 0) |

## Claude Desktop

```json
{
  "mcpServers": {
    "kagi": {
      "command": "/path/to/kagi-mcp/target/release/kagi-mcp",
      "env": {
        "KAGI_CLI_PATH": "/path/to/kagi",
        "KAGI_SESSION_TOKEN": "your-session-token",
        "KAGI_API_TOKEN": "your-api-token"
      }
    }
  }
}
```

## Tools

### Search

| Tool | Description |
|------|-------------|
| `kagi_search` | Search Kagi with optional lens, region, time, and order filters |
| `kagi_batch` | Run multiple searches in parallel with rate limiting |

### Quick Answer

| Tool | Description |
|------|-------------|
| `kagi_quick` | Get a direct answer with references instead of search results |

### Assistant

| Tool | Description |
|------|-------------|
| `kagi_assistant` | Prompt Kagi Assistant, optionally continue an existing thread |
| `kagi_ask_page` | Ask Kagi Assistant about a specific web page |
| `kagi_assistant_thread_list` | List all Assistant threads |
| `kagi_assistant_thread_get` | Get a specific thread by ID |
| `kagi_assistant_thread_export` | Export a thread to markdown or JSON |
| `kagi_assistant_thread_delete` | Delete a thread |

### Translate

| Tool | Description |
|------|-------------|
| `kagi_translate` | Translate text with auto-detection, alternatives, and word insights |

### Summarize

| Tool | Description |
|------|-------------|
| `kagi_summarize` | Summarize URLs or text (subscriber or API mode) |

### News

| Tool | Description |
|------|-------------|
| `kagi_news` | Fetch Kagi News stories by category |
| `kagi_news_categories` | List available news categories |
| `kagi_news_chaos` | Get the current Kagi News chaos index |

### Enrichment

| Tool | Description |
|------|-------------|
| `kagi_enrich_web` | Query Kagi web enrichment index |
| `kagi_enrich_news` | Query Kagi news enrichment index |

### Other

| Tool | Description |
|------|-------------|
| `kagi_smallweb` | Fetch the Kagi Small Web feed |
| `kagi_fastgpt` | Quick factual answers through the paid API |
| `kagi_auth_status` | Show which credentials are configured |
| `kagi_auth_check` | Validate configured credentials |

## Auth Model

| Tool | Session Token | API Token | None |
|------|:---:|:---:|:---:|
| `kagi_search` | ✓ | ✓ | |
| `kagi_search --lens` | ✓ | | |
| `kagi_quick` | ✓ | | |
| `kagi_ask_page` | ✓ | | |
| `kagi_assistant` | ✓ | | |
| `kagi_translate` | ✓ | | |
| `kagi_summarize --subscriber` | ✓ | | |
| `kagi_summarize` | | ✓ | |
| `kagi_fastgpt` | | ✓ | |
| `kagi_enrich_web/news` | | ✓ | |
| `kagi_news` | | | ✓ |
| `kagi_smallweb` | | | ✓ |
| `kagi_auth_status/check` | | | ✓ |

## Architecture

```
MCP Client (Claude, Zed, etc.)
       │ stdio
       ▼
┌─────────────────┐     spawn      ┌───────────┐
│   kagi-mcp      │───────────────▶│  kagi CLI │
│   (rmcp + tokio)│                │           │
└─────────────────┘                └───────────┘
```

The server is a single Rust binary that spawns `kagi` CLI subprocesses for each tool call. JSON output is parsed and forwarded to the MCP client. Text output is passed through as-is.

**Error handling:** CLI subprocess failures (non-zero exit, timeout, invalid JSON) are caught and returned as MCP error results. Timeouts are configurable via `KAGI_MCP_TIMEOUT_MS`.

## Test

```bash
cargo test
```

The test suite uses fixture scripts (mock `kagi` binaries) to verify argument building, JSON parsing, and error surfacing without requiring a real Kagi connection.

## License

MIT
