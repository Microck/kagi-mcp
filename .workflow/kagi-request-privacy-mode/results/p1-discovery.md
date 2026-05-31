# Packet P1 Result

## Accepted
- The repo is a single Rust MCP binary that spawns `kagi` CLI subprocesses.
- `src/main.rs` contains the tool argument structs, CLI argument builders, runner, and unit tests.
- Local `kagi search --help` exposes `--region`, `--personalized`, and `--no-personalized`.
- Local `kagi batch --help` exposes shared `--region`, `--personalized`, `--no-personalized`, `--concurrency`, and `--rate-limit`.
- CloakBrowser is anti-detection oriented and should only inform the boundary of what not to implement.

## Rejected
- Browser automation, fingerprint spoofing, proxies, CAPTCHA avoidance, and bot-detection bypass.

## Decision
Implement `privacy_mode = "unpersonalized"` for `kagi_search` and `kagi_batch`, mapping to `--no-personalized` and defaulting `--region no_region` unless the caller sets a region.
