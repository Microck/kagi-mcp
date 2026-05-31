# kagi request privacy mode

## Goal
Add a transparent privacy-preserving request preset for Kagi search tools without implementing bot-detection evasion, fingerprint spoofing, CAPTCHA avoidance, proxy rotation, or "undetectable" browser automation.

## Success Criteria
- `kagi_search` and `kagi_batch` expose a safe `privacy_mode` option.
- The supported privacy mode uses documented `kagi` CLI flags only.
- The preset disables personalization and defaults region to `no_region` unless the caller explicitly sets a region.
- Existing explicit tool options keep precedence over the preset.
- Tests cover search, batch, and override behavior.
- Rust formatting, tests, and clippy pass.

## Current Context
- This repo is a Rust MCP server that wraps an external `kagi` CLI subprocess.
- Kagi request behavior is delegated to documented CLI flags in `src/main.rs`.
- CloakBrowser's README is explicitly anti-detection oriented, including humanized input, browser fingerprints, CAPTCHA/Cloudflare claims, and proxies. That is useful only as a boundary example of what not to reproduce here.
- The installed `kagi` CLI exposes `--no-personalized`, `--region`, batch `--concurrency`, and batch `--rate-limit`.

## Constraints
- Keep the feature transparent and respectful.
- Do not add dependencies or browser automation.
- Do not handle raw credentials in per-call tool args.
- Do not update README unless explicitly requested.
- Avoid touching existing untracked workflow files outside this run directory.

## Risks
- Naming this as "stealth" would imply bot-detection evasion. Use `privacy_mode` instead.
- The MCP layer cannot change the underlying HTTP/browser transport because it delegates to `kagi`.
- CLI version drift can affect which flags exist, so use already documented flags from local help output.

## Approval Required
No approval is needed for local source edits and tests. Approval would be required before implementing anti-detection evasion, installing CloakBrowser, adding browser automation, using proxies, or changing external systems.

## Work Packets
- P1: Codebase discovery and safe feature shape.
- P2: Implement `privacy_mode` for search and batch argument builders.
- P3: Add focused tests for preset and override behavior.
- P4: Run validators and update final report.

## Integration Policy
Accept only changes that preserve documented CLI delegation and keep user-provided options higher priority than presets. Reject anti-detection, fingerprint, proxy, or CAPTCHA-bypass behavior.

## Verification
- `cargo fmt --check`
- `cargo test`
- `cargo clippy --all-targets --all-features -- -D warnings`

## Reusable Artifacts
This workflow run is specific to this implementation. No reusable recipe is planned unless the final pattern is broadly useful.
