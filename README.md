# kagi-mcp

`kagi-mcp` is a tiny MCP server built on top of [`kagi-cli`](https://github.com/Microck/kagi-cli).

It is intentionally just an extra repo:

- separate repo
- one Rust binary
- wraps the `kagi` CLI instead of reimplementing Kagi logic
- returns TOON by default for structured tool results
- keeps `format=json` available on tools that support CLI format selection

`kagi-cli` v0.5.0 also ships a native `kagi mcp` command. Use that when you only need the minimal built-in tools. Use this repo when you want the broader CLI surface exposed to agents, including Assistant, Extract, News, Translate, enrichment, account settings, local history, local site preferences, watch, and notify workflows.

## Requirements

- `kagi-cli` v0.7.0 or newer
- A working `kagi` binary on `PATH`, or `KAGI_CLI_PATH` pointing to it
- Kagi credentials provided through environment variables
  - `KAGI_SESSION_TOKEN` - for subscriber features (search, quick, assistant, translate)
  - `KAGI_API_KEY` - for current `/api/v1` paid API features (search, extract)
  - `KAGI_API_TOKEN` - for legacy `/api/v0` paid API features (summarize, fastgpt, enrich)

Set `KAGI_CLI_PROFILE` when you want every wrapped CLI call to use a named `.kagi.toml` profile. Environment variables are still the recommended MCP auth path because they are explicit in the MCP server config and do not depend on the server process working directory.

## Build

```bash
cargo build --release
```

## Run

```bash
KAGI_CLI_PATH=/path/to/kagi \
KAGI_SESSION_TOKEN=... \
KAGI_API_KEY=... \
KAGI_API_TOKEN=... \
./target/release/kagi-mcp
```

Optional environment variables:

| Variable | Default | Description |
|----------|---------|-------------|
| `KAGI_CLI_PATH` | `kagi` on `PATH` | Path to the `kagi` binary |
| `KAGI_CLI_PROFILE` | unset | Named `kagi-cli` profile to pass as `kagi --profile <name>` |
| `KAGI_MCP_TIMEOUT_MS` | `30000` | Subprocess timeout in milliseconds, must be greater than 0 |

## Claude Desktop

```json
{
  "mcpServers": {
    "kagi": {
      "command": "/path/to/kagi-mcp/target/release/kagi-mcp",
      "env": {
        "KAGI_CLI_PATH": "/path/to/kagi",
        "KAGI_SESSION_TOKEN": "your-session-token",
        "KAGI_API_KEY": "your-api-key",
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
| `kagi_search` | Search Kagi with optional snap, lens, region, time, date, order, verbatim, personalization, template, follow, limit, news-tab, and local-cache options |
| `kagi_batch` | Run multiple searches in parallel with rate limiting, stdin-style query input, shared search filters, templates, and per-query limits |

### Quick Answer

| Tool | Description |
|------|-------------|
| `kagi_quick` | Get a direct answer with references instead of search results, optionally scoped to a lens |

### Assistant

| Tool | Description |
|------|-------------|
| `kagi_assistant` | Prompt Kagi Assistant, optionally continue an existing thread, attach local files, select a saved assistant, or override model, lens, web access, and personalization |
| `kagi_assistant_repl` | Run a bounded Assistant REPL by feeding prompts through stdin and then exiting |
| `kagi_ask_page` | Ask Kagi Assistant about a specific web page |
| `kagi_assistant_thread_list` | List all Assistant threads |
| `kagi_assistant_thread_get` | Get a specific thread by ID |
| `kagi_assistant_thread_export` | Export a thread to markdown or JSON |
| `kagi_assistant_thread_delete` | Delete a thread |
| `kagi_assistant_custom_list` | List custom and built-in assistants visible to the account |
| `kagi_assistant_custom_get` | Fetch one custom assistant by id or name |
| `kagi_assistant_custom_create` | Create a custom assistant |
| `kagi_assistant_custom_update` | Update a custom assistant by id or name |
| `kagi_assistant_custom_delete` | Delete a custom assistant by id or name |

### Translate

| Tool | Description |
|------|-------------|
| `kagi_translate` | Translate text with auto-detection, alternatives, word insights, and the v0.5.0 text translation controls |

### Summarize

| Tool | Description |
|------|-------------|
| `kagi_summarize` | Summarize URLs or text in subscriber or API mode, or pass `filter_items` to use `kagi summarize --filter` safely through controlled subprocess stdin |

### Extract

| Tool | Description |
|------|-------------|
| `kagi_extract` | Extract a page's full content as markdown through Kagi's paid Extract API, using `KAGI_API_KEY` directly |

### News

| Tool | Description |
|------|-------------|
| `kagi_news` | Fetch Kagi News stories by category, or apply v0.5.0 content-filter presets and keywords |
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
| `kagi_fastgpt` | Quick factual answers through the paid API, with optional local cache controls |
| `kagi_auth_status` | Show which credentials are configured |
| `kagi_auth_check` | Validate configured credentials |
| `kagi_auth_set` | Save API and/or session credentials with `kagi auth set` |
| `kagi_history_list` | List local `kagi-cli` command history entries |
| `kagi_history_stats` | Show aggregate local `kagi-cli` history stats |
| `kagi_site_pref_list` | List local domain preferences |
| `kagi_site_pref_set` | Set a local domain preference: `block`, `lower`, `normal`, `higher`, or `pin` |
| `kagi_site_pref_remove` | Remove a local domain preference |
| `kagi_watch` | Run `kagi watch`; pass a finite `count` to keep the MCP request bounded |
| `kagi_notify` | Run a search or news fetch and post the JSON payload to a webhook |
| `kagi_generate_completion` | Generate a shell completion script for bash, zsh, fish, or PowerShell |

### Account Settings

| Tool | Description |
|------|-------------|
| `kagi_lens_list` | List available lenses |
| `kagi_lens_get` | Fetch one lens by id or exact name |
| `kagi_lens_create` | Create a lens |
| `kagi_lens_update` | Update a lens by id or exact name |
| `kagi_lens_delete` | Delete a lens by id or exact name |
| `kagi_lens_enable` | Enable a lens by id or exact name |
| `kagi_lens_disable` | Disable a lens by id or exact name |
| `kagi_bang_custom_list` | List custom bangs |
| `kagi_bang_custom_get` | Fetch one custom bang by id, name, or trigger |
| `kagi_bang_custom_create` | Create a custom bang |
| `kagi_bang_custom_update` | Update a custom bang by id, name, or trigger |
| `kagi_bang_custom_delete` | Delete a custom bang by id, name, or trigger |
| `kagi_redirect_list` | List redirect rules |
| `kagi_redirect_get` | Fetch one redirect rule by id or exact rule text |
| `kagi_redirect_create` | Create a redirect rule |
| `kagi_redirect_update` | Update a redirect rule by id or exact rule text |
| `kagi_redirect_delete` | Delete a redirect rule by id or exact rule text |
| `kagi_redirect_enable` | Enable a redirect rule by id or exact rule text |
| `kagi_redirect_disable` | Disable a redirect rule by id or exact rule text |

## Scope Boundaries

`kagi_watch` is exposed for parity, but it can run until the configured subprocess timeout if `count` is omitted or set to `0`. Prefer a finite `count` for MCP calls. `kagi_assistant_repl` is exposed as a bounded REPL wrapper: pass `prompts`, and the server feeds those prompts to `kagi assistant repl` followed by `/exit`. `kagi_notify` and account-setting mutation tools are side-effecting; call them only when the webhook or account mutation is intentional.

The wrapped CLI's `kagi mcp` subcommand is not exposed as a tool because this binary is already the MCP server. Spawning another stdio MCP server inside one tool call would not produce a normal request/response result.

## Auth Model

| Tool | Session Token | API Key | API Token | None |
|------|:---:|:---:|:---:|:---:|
| `kagi_search` | yes | yes | | |
| `kagi_search --lens` | yes | | | |
| `kagi_quick` | yes | | | |
| `kagi_ask_page` | yes | | | |
| `kagi_assistant` | yes | | | |
| `kagi_assistant_repl` | yes | | | |
| `kagi_translate` | yes | | | |
| `kagi_summarize --subscriber` | yes | | | |
| `kagi_summarize` | | | yes | |
| `kagi_extract` | | yes | | |
| `kagi_fastgpt` | | | yes | |
| `kagi_enrich_web/news` | | | yes | |
| `kagi_lens_*` | yes | | | |
| `kagi_bang_custom_*` | yes | | | |
| `kagi_redirect_*` | yes | | | |
| `kagi_assistant_custom_*` | yes | | | |
| `kagi_watch` | yes | yes | | |
| `kagi_notify` | yes | yes | | |
| `kagi_news` | | | | yes |
| `kagi_smallweb` | | | | yes |
| `kagi_history_list/stats` | | | | yes |
| `kagi_site_pref_*` | | | | yes |
| `kagi_generate_completion` | | | | yes |
| `kagi_auth_status/check/set` | | | | yes |

## Architecture

```text
MCP Client (Claude, Zed, etc.)
       | stdio
       v
+-----------------+     spawn      +-----------+
|   kagi-mcp      |--------------->|  kagi CLI |
|   rmcp + tokio  |                |           |
+-----------------+                +-----------+
```

The server is a single Rust binary that spawns `kagi` CLI subprocesses for each tool call. Structured JSON output is parsed and converted to TOON by default; explicit text output is passed through as-is.

**Error handling:** CLI subprocess failures, non-zero exits, timeouts, and invalid JSON are returned as MCP error results. Timeouts are configurable via `KAGI_MCP_TIMEOUT_MS`.

## Test

```bash
cargo test
cargo clippy --all-targets --all-features -- -D warnings
```

The test suite uses fixture scripts as local `kagi` binaries to verify argument building, JSON parsing, TOON conversion, stdin handling, profile passthrough, and error surfacing without requiring a real Kagi connection.

## License

MIT
