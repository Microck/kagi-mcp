# Orchestration: kagi request privacy mode

## Execution Rules

- Keep the original objective intact.
- Ask for approval before risky, expensive, external, or destructive actions.
- Keep immediate blocking work local.
- Delegate only bounded, disjoint, materially useful packets.
- Integrate packet results before final verification.
- Treat "stealth" as an unsafe naming and implementation direction. Implement only transparent privacy controls.

## Branching Rules
- If the implementation requires browser automation, fingerprint patches, proxies, CAPTCHA avoidance, or evasion of bot detection, stop and ask for explicit approval after explaining the risk.
- If `kagi` CLI help lacks a candidate flag, do not invent it at the MCP layer.
- If tests fail because of unrelated existing workspace changes, report the blocker before broadening scope.

## Packet Prompts
- P1 discovery: inspect `src/main.rs`, `Cargo.toml`, README test instructions, and local `kagi` help for safe documented flags.
- P2 implementation: add a `privacy_mode` field and helper that maps the preset to existing CLI flags.
- P3 tests: cover search, batch, explicit region preservation, and explicit personalization preservation.
- P4 verification: run formatting, unit tests, clippy, and workflow artifact validation.

## Completion Audit
- Source changes are limited to `src/main.rs`.
- Workflow artifacts are updated under `.workflow/kagi-request-privacy-mode`.
- Validators pass or failures are reported with exact diagnostics.
