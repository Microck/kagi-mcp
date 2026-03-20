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
  - `KAGI_SESSION_TOKEN` - for subscriber features (search, quick, assistant, translate, etc.)
  - `KAGI_API_TOKEN` - for paid API features (summarize, fastgpt, enrich)

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

- `kagi_search` - Search Kagi with optional lens, region, time, and order filters
- `kagi_batch` - Run multiple searches in parallel with rate limiting

### Quick Answer

- `kagi_quick` - Get a direct answer with references instead of search results

### Assistant

- `kagi_assistant` - Prompt Kagi Assistant, optionally continue an existing thread
- `kagi_ask_page` - Ask Kagi Assistant about a specific web page
- `kagi_assistant_thread_list` - List all Assistant threads
- `kagi_assistant_thread_get` - Get a specific thread by ID
- `kagi_assistant_thread_export` - Export a thread to markdown or JSON
- `kagi_assistant_thread_delete` - Delete a thread

### Translate

- `kagi_translate` - Translate text with auto-detection, alternatives, and word insights

### Summarize

- `kagi_summarize` - Summarize URLs or text (subscriber or API mode)

### News

- `kagi_news` - Fetch Kagi News stories by category
- `kagi_news_categories` - List available news categories
- `kagi_news_chaos` - Get the current Kagi News chaos index

### Enrichment

- `kagi_enrich_web` - Query Kagi web enrichment index
- `kagi_enrich_news` - Query Kagi news enrichment index

### Other

- `kagi_smallweb` - Fetch the Kagi Small Web feed
- `kagi_fastgpt` - Quick factual answers through the paid API
- `kagi_auth_status` - Show which credentials are configured
- `kagi_auth_check` - Validate configured credentials

## Auth Model

| Tool | Session Token | API Token | None |
|------|---------------|-----------|------|
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

## Test

```bash
cargo test
```