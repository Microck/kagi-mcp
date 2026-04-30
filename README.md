# kagi-mcp

`kagi-mcp` is a tiny MCP server built on top of [`kagi-cli`](https://github.com/Microck/kagi-cli).

It is intentionally just an extra repo:

- separate repo
- one Rust binary
- wraps the `kagi` CLI instead of reimplementing Kagi logic
- returns the same JSON the CLI already emits

`kagi-cli` v0.5.0 also ships a native `kagi mcp` command. Use that when you only need the minimal built-in tools (`kagi_search`, `kagi_summarize`, and `kagi_quick`). Use this repo when you want the broader CLI surface exposed to agents, including Assistant, News, Translate, enrichment, local history, and local site preferences.

## Requirements

- `kagi-cli` v0.5.0 or newer
- A working `kagi` binary on `PATH`, or `KAGI_CLI_PATH` pointing to it
- Kagi credentials provided through environment variables
  - `KAGI_SESSION_TOKEN` - for subscriber features (search, quick, assistant, translate, etc.)
  - `KAGI_API_TOKEN` - for paid API features (summarize, fastgpt, enrich)

Set `KAGI_CLI_PROFILE` when you want every wrapped CLI call to use a named `.kagi.toml` profile. Environment variables are still the recommended MCP auth path because they are explicit in the MCP server config and do not depend on the server process working directory.

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
- `KAGI_CLI_PROFILE`: named `kagi-cli` profile to pass as `kagi --profile <name>`
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

- `kagi_search` - Search Kagi with optional snap, lens, region, time, date, order, verbatim, personalization, template, follow, and local-cache options
- `kagi_batch` - Run multiple searches in parallel with rate limiting, stdin-style query input, and shared search filters

### Quick Answer

- `kagi_quick` - Get a direct answer with references instead of search results, optionally scoped to a lens

### Assistant

- `kagi_assistant` - Prompt Kagi Assistant, optionally continue an existing thread, attach local files, select a saved assistant, or override model, lens, web access, and personalization
- `kagi_ask_page` - Ask Kagi Assistant about a specific web page
- `kagi_assistant_thread_list` - List all Assistant threads
- `kagi_assistant_thread_get` - Get a specific thread by ID
- `kagi_assistant_thread_export` - Export a thread to markdown or JSON
- `kagi_assistant_thread_delete` - Delete a thread

### Translate

- `kagi_translate` - Translate text with auto-detection, alternatives, word insights, and the v0.5.0 text translation controls

### Summarize

- `kagi_summarize` - Summarize URLs or text (subscriber or API mode), or pass `filter_items` to use `kagi summarize --filter` safely through controlled subprocess stdin

### News

- `kagi_news` - Fetch Kagi News stories by category, or apply v0.5.0 content-filter presets and keywords
- `kagi_news_categories` - List available news categories
- `kagi_news_chaos` - Get the current Kagi News chaos index

### Enrichment

- `kagi_enrich_web` - Query Kagi web enrichment index
- `kagi_enrich_news` - Query Kagi news enrichment index

### Other

- `kagi_smallweb` - Fetch the Kagi Small Web feed
- `kagi_fastgpt` - Quick factual answers through the paid API, with optional local cache controls
- `kagi_auth_status` - Show which credentials are configured
- `kagi_auth_check` - Validate configured credentials
- `kagi_history_list` - List local `kagi-cli` command history entries
- `kagi_history_stats` - Show aggregate local `kagi-cli` history stats
- `kagi_site_pref_list` - List local domain preferences
- `kagi_site_pref_set` - Set a local domain preference (`block`, `lower`, `normal`, `higher`, or `pin`)
- `kagi_site_pref_remove` - Remove a local domain preference

## Scope Boundaries

`kagi_watch` and `kagi_notify` are not exposed by this server. `watch` is a long-running polling workflow, which does not fit well inside a request/response MCP tool. `notify` posts to external webhooks, so keeping it as an explicit CLI command avoids surprising agent-triggered side effects.

The account-settings management commands for lenses, custom bangs, redirects, and custom assistants remain available through the wrapped `kagi` CLI. This server focuses on safe request/response tools plus local-state inspection and preferences.

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
| `kagi_history_list/stats` | | | ✓ |
| `kagi_site_pref_*` | | | ✓ |
| `kagi_auth_status/check` | | | ✓ |

## Test

```bash
cargo test
cargo clippy --all-targets --all-features -- -D warnings
```
