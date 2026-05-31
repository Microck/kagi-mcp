# Packet P2: Implementation

## Objective
Add a transparent `privacy_mode` preset for search-oriented MCP tools.

## Context
`kagi-mcp` delegates request behavior to the external `kagi` CLI. The MCP layer should only translate structured tool arguments into documented CLI flags.

## Ownership
`src/main.rs`

## Do
- Add a `PrivacyMode` enum with an `unpersonalized` serialized value.
- Add `privacy_mode` to `SearchArgs` and `BatchArgs`.
- Apply the preset before CLI argument construction.
- Preserve explicit caller options when they conflict with preset defaults.
- Add focused unit tests.

## Do Not
- Add dependencies.
- Add browser automation or stealth/fingerprint code.
- Change README unless explicitly requested.

## Verification
Run `cargo fmt --check`, `cargo test`, and `cargo clippy --all-targets --all-features -- -D warnings`.
