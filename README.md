# kagi-mcp

[![Crates.io](https://img.shields.io/crates/v/kagi-mcp)](https://crates.io/crates/kagi-mcp)
[![License: MIT](https://img.shields.io/badge/License-MIT-yellow.svg)](https://opensource.org/licenses/MIT)
[![Rust](https://img.shields.io/badge/Rust-1.85+-dea584.svg?logo=rust)](https://www.rust-lang.org)

`kagi-mcp` is a tiny MCP server that provides access to [Kagi](https://kagi.com) search and AI services through the Model Context Protocol. It wraps the [`kagi-cli`](https://github.com/Microck/kagi-cli) to offer search, summarization, translation, assistant interactions, and more to MCP clients like Claude Desktop, Zed, and others.

## Why a Separate Repo?

This project is intentionally separate from `kagi-cli`:

- Single Rust binary, easy to distribute
- Wraps the `kagi` CLI instead of reimplementing Kagi logic
- Returns the same JSON the CLI already emits
- Minimal dependencies, focused on MCP protocol handling

## Requirements

- A working `kagi` binary on `PATH`, or `KAGI_CLI_PATH` pointing to it
- Kagi credentials provided through environment variables
- `KAGI_SESSION_TOKEN` — for subscriber features (search, quick, assistant, translate, etc.)
- `KAGI_API_TOKEN` — for paid API features (summarize, fastgpt, enrich)

> **Note:** `.kagi.toml` is not the recommended auth path for MCP usage because the CLI resolves it relative to the server process working directory.

### Getting Your Tokens

1. **Session Token** (subscriber features): Available in your [Kagi account settings](https://kagi.com/settings)
2. **API Token** (paid API features): Generate in your [Kagi developer settings](https://kagi.com/settings?tab=developer)

## Features

- **Search** — Full-text web search with filters (lens, region, time, ordering)
- **Quick Answer** — Direct answers with source citations
- **Assistant** — Conversational AI with thread management
- **Summarize** — URL or text summarization (subscriber or API mode)
- **Translate** — Auto-detect language, alternatives, word-level insights
- **News** — Kagi News by category, including chaos index
- **Enrichment** — Web and news enrichment queries
- **FastGPT** — Quick factual answers via paid API
- **Small Web** — Curated small web feed

## Install kagi-cli

The server requires the `kagi` CLI to be installed. Install it via:

```bash
# Using Cargo
cargo install kagi-cli

# Or download pre-built binaries from the releases page
# https://github.com/Microck/kagi-cli/releases
```

Verify the installation:

```bash
kagi --version
```

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

Add the following to your Claude Desktop config file (`claude_desktop_config.json`):

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

### Zed

Add to your Zed `settings.json`:

```json
{
  "model_context_providers": {
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
