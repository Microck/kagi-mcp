# Changelog

All notable changes to this project will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [0.1.0] - 2025-03-17

### Added

- Initial MCP server wrapping `kagi-cli` (`rmcp`-based, stdio transport)
- `search` tool — Kagi search via CLI JSON output
- `summarize` tool — URL/text summarization via Kagi summarizer
- `fastgpt` tool — FastGPT quick answers
- `enrich_web` / `enrich_news` tools — Kagi enrichment endpoints
- Environment variable configuration (`KAGI_SESSION_TOKEN`, `KAGI_API_KEY`, `KAGI_CLI_PATH`)
- Basic error handling with `thiserror`-based error types

## [0.2.0] - 2025-03-20

### Added

- `quick` tool — Kagi Quick Answer with references
- `translate` tool — Kagi Translate with alignment/alternative support
- `batch` tool — parallel multi-query search with rate limiting
- `ask_page` tool — ask questions about a specific URL
- Thread management tools: `assistant`, `assistant_thread_list`, `assistant_thread_get`, `assistant_thread_export`, `assistant_thread_delete`
- `news` / `news_categories` / `news_chaos` tools — Kagi News feed
- `smallweb` tool — Kagi Small Web feed
- `summarize` tool — expanded with subscriber-mode and cache options

## [Unreleased]

### Added

- README improvements: architecture diagram, environment variable table, tool tables, license section ([#2](https://github.com/Microck/kagi-mcp/pull/2))
