# Repository guidance

- Preserve the user constraints recorded in `docs/DECISIONS.md`.
- Before changing planning, quota, reset, preference, or progress semantics, read `docs/MATHEMATICS.md`; update it together with implementation and meaningful regression tests.
- Keep each model's quota independent. Tibo is an observed random refresh, never a stored reset card or forecasted quota.
- Token consumption is not task progress. Never infer progress automatically from usage.
- Daily operations are local. Explicit hub push/pull/clone and project checkout may use Git networking, as requested. Never store credentials or perform actual provider account resets.
- Manage usage only, never money or pricing. Each model's API token cap is independent; zero forbids API planning. Keep subscription and API ledgers separate.
- Before committing, run `cargo fmt --check`, `cargo test --locked`, and `cargo clippy --locked --all-targets -- -D warnings`.
- Do not commit personal data directories or compiled binaries. `Cargo.lock` is tracked.
- When outputting math in chat or documents, use `$...$` for inline formulas and `$$...$$` for display formulas.
