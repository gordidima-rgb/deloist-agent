# Contributing to Deloist

## Build

- Install Rust with `rustup` only; `rust-toolchain.toml` pins the toolchain.
- `cargo build --examples` builds the helper binaries the integration tests need (for example `fake_acp`).
- `cargo test` runs the full suite. While working, run just your target: `cargo test --test <name>` for integration tests, `cargo test --bin deloist <module>::` for unit tests.
- UI (Svelte): `cd ui && npm ci && npm run build && npm test`. After changing `ui/`, rebuild so `web/ui/` is up to date (`node tools/check_ui_built.mjs`).
- Line endings are LF everywhere except `*.ps1`, `*.cmd`, `*.bat` (CRLF); see `.gitattributes`.

## Parts not in this repository

Some parts of the official Deloist build (Deloist account and subscription, the full catalog, glass-orb avatars) are not in this repository; without them the app uses stubs and runs fully with your own models.

## Security rules

Any new API route that changes commands, addresses, keys or consents must be human-only: add it to `guard::human_only` and cover it with a test in `tests/security.rs`. Bot scripts can read the program key, so never trust a request just because it carries it.

## Licence of contributions

By submitting a contribution you grant the copyright holder a perpetual, worldwide, royalty-free licence to use, modify and distribute it under PolyForm Noncommercial 1.0.0 and under any other licence, including commercial terms for official builds.

## Contact

Security reports: send them privately to support@deloist.com, not in public issues.
Commercial licensing enquiries: support@deloist.com.
