# Packet P2 Result

## Accepted
- Added `PrivacyMode` with serialized value `unpersonalized`.
- Added `privacy_mode` to `SearchArgs` and `BatchArgs`.
- Added `apply_privacy_mode` so the preset defaults region to `no_region` and disables personalization.
- Preserved explicit `region` and explicit `personalized = true` options.
- Avoided adding `--no-personalized` to news-tab searches, where the local CLI help says that flag conflicts.
- Added tests for search and batch preset behavior.

## Rejected
- No browser, proxy, CAPTCHA, fingerprint, or anti-detection behavior was added.

## Verification
- `cargo test` passed with 31 tests.
- `cargo fmt --check` passed.
- `cargo clippy --all-targets --all-features -- -D warnings` passed.
